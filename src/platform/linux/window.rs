use super::{MonitorInfo, PlatformError};
use crossbeam::channel::{unbounded, Receiver, Sender};
use raw_window_handle::{
    HasRawDisplayHandle,
    HasRawWindowHandle,
    RawDisplayHandle,
    RawWindowHandle,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, Event, KeyEvent, MouseButton, WindowEvent as WinitWindowEvent};
use winit::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use winit::keyboard::PhysicalKey;
use winit::platform::unix::EventLoopBuilderExtUnix;
use winit::window::WindowBuilder;

/// Maximum number of events drained per poll iteration to avoid starving the dispatcher.
const MAX_EVENT_BATCH: usize = 64;

/// Commands sent from the engine to the window thread.
#[derive(Debug)]
enum LoopCommand {
    SetTitle(String),
    SetSize(u32, u32),
    SetVisible(bool),
    Exit,
}

/// Linux window implementation backed by winit to support both X11 and Wayland compositors.
pub struct LinuxWindow {
    width: u32,
    height: u32,
    title: String,
    raw_window_handle: RawWindowHandle,
    raw_display_handle: RawDisplayHandle,
    event_rx: Receiver<WindowEvent>,
    loop_exit_rx: Receiver<()>,
    command_proxy: EventLoopProxy<LoopCommand>,
    event_thread: Option<JoinHandle<()>>,
    is_visible: AtomicBool,
}

impl LinuxWindow {
    pub fn new(width: u32, height: u32, title: &str) -> Result<Self, PlatformError> {
        static EVENT_LOOP_CREATED: OnceLock<()> = OnceLock::new();

        // Ensure only one window loop is created per process for now.
        if EVENT_LOOP_CREATED.set(()).is_err() {
            return Err(PlatformError::WindowCreationFailed(
                "Only a single Linux window is supported at the moment".to_string(),
            ));
        }

        let (event_tx, event_rx) = unbounded::<WindowEvent>();
        let (ready_tx, ready_rx) = unbounded();
        let (loop_exit_tx, loop_exit_rx) = unbounded::<()>();

        let width = width.max(1);
        let height = height.max(1);
        let title = title.to_string();

        let window_thread = thread::Builder::new()
            .name("vk-browser-linux-window".to_string())
            .spawn(move || {
                if let Err(err) = Self::run_event_loop(
                    width,
                    height,
                    title,
                    event_tx,
                    ready_tx,
                    loop_exit_tx,
                ) {
                    tracing::error!("Window loop terminated with error: {}", err);
                }
            })
            .map_err(|e| PlatformError::WindowCreationFailed(e.to_string()))?;

        let ready = ready_rx
            .recv()
            .map_err(|_| PlatformError::WindowCreationFailed("Window thread failed to start".into()))?;

        let (command_proxy, raw_window_handle, raw_display_handle) = ready;

        Ok(Self {
            width,
            height,
            title,
            raw_window_handle,
            raw_display_handle,
            event_rx,
            loop_exit_rx,
            command_proxy,
            event_thread: Some(window_thread),
            is_visible: AtomicBool::new(true),
        })
    }

    fn run_event_loop(
        width: u32,
        height: u32,
        title: String,
        event_tx: Sender<WindowEvent>,
        ready_tx: Sender<(EventLoopProxy<LoopCommand>, RawWindowHandle, RawDisplayHandle)>,
        loop_exit_tx: Sender<()>,
    ) -> Result<(), PlatformError> {
        // Allow building the event loop off the main thread.
        let mut builder = EventLoopBuilder::<LoopCommand>::with_user_event();
        builder.with_any_thread(true);
        let event_loop = builder
            .build()
            .map_err(|e| PlatformError::WindowCreationFailed(format!("Failed to build event loop: {e}")))?;

        let window = WindowBuilder::new()
            .with_title(&title)
            .with_inner_size(PhysicalSize::new(width, height))
            .build(&event_loop)
            .map_err(|e| PlatformError::WindowCreationFailed(format!("Failed to create window: {e}")))?;

        window.set_visible(true);

        let command_proxy = event_loop.create_proxy();
        let raw_window_handle = window.raw_window_handle();
        let raw_display_handle = window.raw_display_handle();

        ready_tx
            .send((command_proxy.clone(), raw_window_handle, raw_display_handle))
            .map_err(|_| PlatformError::WindowCreationFailed("Failed to notify window readiness".into()))?;

        // Keep the proxy alive inside the event loop scope to allow user commands.
        let _command_proxy = command_proxy;

        let mut cursor_position = (0.0f64, 0.0f64);

        event_loop.run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::Wait);

            match event {
                Event::UserEvent(command) => {
                    match command {
                        LoopCommand::SetTitle(new_title) => window.set_title(&new_title),
                        LoopCommand::SetSize(new_width, new_height) => {
                            window.set_inner_size(PhysicalSize::new(new_width, new_height));
                        }
                        LoopCommand::SetVisible(visible) => window.set_visible(visible),
                        LoopCommand::Exit => {
                            let _ = event_tx.send(WindowEvent::Closed);
                            elwt.exit();
                        }
                    }
                }
                Event::WindowEvent { event, .. } => match event {
                    WinitWindowEvent::CloseRequested => {
                        let _ = event_tx.send(WindowEvent::Closed);
                        elwt.exit();
                    }
                    WinitWindowEvent::RedrawRequested => {
                        let _ = event_tx.send(WindowEvent::Redraw);
                    }
                    WinitWindowEvent::Resized(new_size) => {
                        let _ = event_tx.send(WindowEvent::Resized {
                            width: new_size.width,
                            height: new_size.height,
                        });
                    }
                    WinitWindowEvent::KeyboardInput { event: key_event, .. } => {
                        if key_event.state == ElementState::Pressed {
                            if let Some(window_event) = Self::translate_key_event(&key_event) {
                                let _ = event_tx.send(window_event);
                            }
                        }
                    }
                    WinitWindowEvent::MouseInput { state, button, .. } => {
                        if state == ElementState::Pressed {
                            let (x, y) = cursor_position;
                            let button_id = match button {
                                MouseButton::Left => 1,
                                MouseButton::Right => 2,
                                MouseButton::Middle => 3,
                                MouseButton::Other(other) => other,
                            };
                            let _ = event_tx.send(WindowEvent::MousePressed {
                                x,
                                y,
                                button: button_id,
                            });
                        }
                    }
                    WinitWindowEvent::CursorMoved { position, .. } => {
                        cursor_position = (position.x, position.y);
                    }
                    _ => {}
                },
                Event::AboutToWait => window.request_redraw(),
                Event::LoopDestroyed => {
                    let _ = loop_exit_tx.send(());
                    let _ = event_tx.send(WindowEvent::Closed);
                }
                _ => {}
            }
        });
    }

    fn translate_key_event(key_event: &KeyEvent) -> Option<WindowEvent> {
        match key_event.physical_key {
            PhysicalKey::Code(code) => Some(WindowEvent::KeyPressed {
                keycode: code as u32,
                modifiers: key_event.modifiers.state().bits() as u32,
            }),
            _ => None,
        }
    }

    pub fn poll_events(&self) -> Vec<WindowEvent> {
        let mut events = Vec::new();
        for _ in 0..MAX_EVENT_BATCH {
        match self.event_rx.try_recv() {
                Ok(WindowEvent::Resized { width, height }) => {
                    self.width = width;
                    self.height = height;
                    events.push(WindowEvent::Resized { width, height });
                }
                Ok(WindowEvent::Closed) => {
                    self.is_visible.store(false, Ordering::SeqCst);
                    events.push(WindowEvent::Closed);
                }
                Ok(event) => events.push(event),
                Err(_) => break,
            }
        }
        events
    }

    pub fn get_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.width = width;
        self.height = height;
        let _ = self.command_proxy.send_event(LoopCommand::SetSize(width, height));
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
        let _ = self
            .command_proxy
            .send_event(LoopCommand::SetTitle(self.title.clone()));
    }

    pub fn show(&self) {
        if self
            .is_visible
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let _ = self.command_proxy.send_event(LoopCommand::SetVisible(true));
        }
    }

    pub fn hide(&self) {
        if self
            .is_visible
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let _ = self
                .command_proxy
                .send_event(LoopCommand::SetVisible(false));
        }
    }

    pub fn get_vulkan_surface_extensions(&self) -> Vec<&'static str> {
        let mut extensions = vec!["VK_KHR_surface"];
        match self.raw_window_handle {
            RawWindowHandle::Wayland(_) => extensions.push("VK_KHR_wayland_surface"),
            RawWindowHandle::Xlib(_) => extensions.push("VK_KHR_xlib_surface"),
            RawWindowHandle::Xcb(_) => extensions.push("VK_KHR_xcb_surface"),
            RawWindowHandle::Win32(_) => {}
            RawWindowHandle::UiKit(_)
            | RawWindowHandle::AppKit(_)
            | RawWindowHandle::Web(_) => {}
            _ => {}
        }
        extensions
    }
}

impl Drop for LinuxWindow {
    fn drop(&mut self) {
        let _ = self.command_proxy.send_event(LoopCommand::Exit);
        // Wait for loop exit signal to avoid leaking the window thread.
        let _ = self.loop_exit_rx.recv_timeout(Duration::from_secs(1));
        if let Some(handle) = self.event_thread.take() {
            let _ = handle.join();
        }
    }
}

unsafe impl HasRawWindowHandle for LinuxWindow {
    fn raw_window_handle(&self) -> RawWindowHandle {
        self.raw_window_handle
    }
}

unsafe impl HasRawDisplayHandle for LinuxWindow {
    fn raw_display_handle(&self) -> RawDisplayHandle {
        self.raw_display_handle
    }
}

#[derive(Debug, Clone)]
pub enum WindowEvent {
    Redraw,
    KeyPressed { keycode: u32, modifiers: u32 },
    MousePressed { x: f64, y: f64, button: u32 },
    Resized { width: u32, height: u32 },
    Closed,
}
