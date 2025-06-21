use super::{PlatformError, MonitorInfo};
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle, XlibWindowHandle, XlibDisplayHandle};
use std::ffi::CString;
use std::ptr;

pub struct LinuxWindow {
    display: *mut x11::xlib::Display,
    window: x11::xlib::Window,
    width: u32,
    height: u32,
    title: String,
}

impl LinuxWindow {
    pub fn new(width: u32, height: u32, title: &str) -> Result<Self, PlatformError> {
        unsafe {
            let display = x11::xlib::XOpenDisplay(ptr::null());
            if display.is_null() {
                return Err(PlatformError::WindowCreationFailed("Failed to open X11 display".to_string()));
            }

            let screen = x11::xlib::XDefaultScreen(display);
            let root_window = x11::xlib::XRootWindow(display, screen);

            let window = x11::xlib::XCreateSimpleWindow(
                display,
                root_window,
                0, 0,
                width, height,
                1,
                x11::xlib::XBlackPixel(display, screen),
                x11::xlib::XWhitePixel(display, screen),
            );

            let title_cstring = CString::new(title).unwrap();
            x11::xlib::XStoreName(display, window, title_cstring.as_ptr());

            x11::xlib::XSelectInput(
                display,
                window,
                x11::xlib::ExposureMask | x11::xlib::KeyPressMask | x11::xlib::ButtonPressMask | x11::xlib::StructureNotifyMask,
            );

            x11::xlib::XMapWindow(display, window);
            x11::xlib::XFlush(display);

            Ok(Self {
                display,
                window,
                width,
                height,
                title: title.to_string(),
            })
        }
    }

    pub fn poll_events(&self) -> Vec<WindowEvent> {
        let mut events = Vec::new();
        unsafe {
            let mut count = x11::xlib::XPending(self.display);
            while count > 0 {
                let mut event: x11::xlib::XEvent = std::mem::zeroed();
                x11::xlib::XNextEvent(self.display, &mut event);

                match event.get_type() {
                    x11::xlib::Expose => {
                        events.push(WindowEvent::Redraw);
                    }
                    x11::xlib::KeyPress => {
                        let key_event = event.key;
                        events.push(WindowEvent::KeyPressed {
                            keycode: key_event.keycode,
                            modifiers: key_event.state,
                        });
                    }
                    x11::xlib::ButtonPress => {
                        let button_event = event.button;
                        events.push(WindowEvent::MousePressed {
                            x: button_event.x as f64,
                            y: button_event.y as f64,
                            button: button_event.button,
                        });
                    }
                    x11::xlib::ConfigureNotify => {
                        let configure_event = event.configure;
                        events.push(WindowEvent::Resized {
                            width: configure_event.width as u32,
                            height: configure_event.height as u32,
                        });
                    }
                    _ => {}
                }

                count = x11::xlib::XPending(self.display);
            }
        }
        events
    }

    pub fn get_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        unsafe {
            x11::xlib::XResizeWindow(self.display, self.window, width, height);
            x11::xlib::XFlush(self.display);
        }
        self.width = width;
        self.height = height;
    }

    pub fn set_title(&mut self, title: &str) {
        let title_cstring = CString::new(title).unwrap();
        unsafe {
            x11::xlib::XStoreName(self.display, self.window, title_cstring.as_ptr());
            x11::xlib::XFlush(self.display);
        }
        self.title = title.to_string();
    }

    pub fn show(&self) {
        unsafe {
            x11::xlib::XMapWindow(self.display, self.window);
            x11::xlib::XFlush(self.display);
        }
    }

    pub fn hide(&self) {
        unsafe {
            x11::xlib::XUnmapWindow(self.display, self.window);
            x11::xlib::XFlush(self.display);
        }
    }

    pub fn get_vulkan_surface_extensions(&self) -> Vec<&'static str> {
        vec!["VK_KHR_surface", "VK_KHR_xlib_surface"]
    }
}

unsafe impl HasRawWindowHandle for LinuxWindow {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let mut handle = XlibWindowHandle::empty();
        handle.window = self.window;
        RawWindowHandle::Xlib(handle)
    }
}

impl Drop for LinuxWindow {
    fn drop(&mut self) {
        unsafe {
            x11::xlib::XDestroyWindow(self.display, self.window);
            x11::xlib::XCloseDisplay(self.display);
        }
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