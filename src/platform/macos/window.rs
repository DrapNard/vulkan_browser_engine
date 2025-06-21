use super::{PlatformError, MonitorInfo};
use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicyRegular, NSWindow, NSWindowStyleMask, NSBackingStoreBuffered};
use cocoa::base::{id, nil, YES, NO};
use cocoa::foundation::{NSRect, NSPoint, NSSize, NSString, NSAutoreleasePool};
use core_graphics::geometry::CGRect;
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle, AppKitWindowHandle};
use std::sync::Once;

static INIT: Once = Once::new();

pub struct MacOSWindow {
    window: id,
    width: u32,
    height: u32,
    title: String,
}

impl MacOSWindow {
    pub fn new(width: u32, height: u32, title: &str) -> Result<Self, PlatformError> {
        INIT.call_once(|| {
            unsafe {
                let app = NSApp();
                app.setActivationPolicy_(NSApplicationActivationPolicyRegular);
            }
        });

        unsafe {
            let pool = NSAutoreleasePool::new(nil);

            let window_rect = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(width as f64, height as f64),
            );

            let window = NSWindow::alloc(nil)
                .initWithContentRect_styleMask_backing_defer_(
                    window_rect,
                    NSWindowStyleMask::NSTitledWindowMask
                        | NSWindowStyleMask::NSClosableWindowMask
                        | NSWindowStyleMask::NSMiniaturizableWindowMask
                        | NSWindowStyleMask::NSResizableWindowMask,
                    NSBackingStoreBuffered,
                    NO,
                );

            let title_string = NSString::alloc(nil).init_str(title);
            window.setTitle_(title_string);
            window.center();
            window.makeKeyAndOrderFront_(nil);

            pool.drain();

            Ok(Self {
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
            let pool = NSAutoreleasePool::new(nil);
            
            let app = NSApp();
            let event = app.nextEventMatchingMask_untilDate_inMode_dequeue_(
                std::u64::MAX,
                nil,
                cocoa::appkit::NSDefaultRunLoopMode,
                YES,
            );

            if !event.is_null() {
                let event_type = event.eventType();
                match event_type {
                    1 => { // NSLeftMouseDown
                        let location = event.locationInWindow();
                        events.push(WindowEvent::MousePressed {
                            x: location.x,
                            y: location.y,
                            button: 1,
                        });
                    }
                    10 => { // NSKeyDown
                        let keycode = event.keyCode() as u32;
                        events.push(WindowEvent::KeyPressed {
                            keycode,
                            modifiers: 0,
                        });
                    }
                    _ => {}
                }
                app.sendEvent_(event);
            }

            pool.drain();
        }
        events
    }

    pub fn get_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        unsafe {
            let frame = self.window.frame();
            let new_frame = NSRect::new(
                frame.origin,
                NSSize::new(width as f64, height as f64),
            );
            self.window.setFrame_display_(new_frame, YES);
        }
        self.width = width;
        self.height = height;
    }

    pub fn set_title(&mut self, title: &str) {
        unsafe {
            let title_string = NSString::alloc(nil).init_str(title);
            self.window.setTitle_(title_string);
        }
        self.title = title.to_string();
    }

    pub fn show(&self) {
        unsafe {
            self.window.makeKeyAndOrderFront_(nil);
        }
    }

    pub fn hide(&self) {
        unsafe {
            self.window.orderOut_(nil);
        }
    }

    pub fn get_vulkan_surface_extensions(&self) -> Vec<&'static str> {
        vec!["VK_KHR_surface", "VK_MVK_macos_surface"]
    }
}

unsafe impl HasRawWindowHandle for MacOSWindow {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let mut handle = AppKitWindowHandle::empty();
        handle.ns_window = self.window as *mut std::ffi::c_void;
        handle.ns_view = unsafe { self.window.contentView() } as *mut std::ffi::c_void;
        RawWindowHandle::AppKit(handle)
    }
}

impl Drop for MacOSWindow {
    fn drop(&mut self) {
        unsafe {
            self.window.close();
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
