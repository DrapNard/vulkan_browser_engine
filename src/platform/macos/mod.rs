pub mod window;

pub use window::*;

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::display::{CGDisplay, CGMainDisplayID};

pub struct MacOSPlatform;

impl MacOSPlatform {
    pub fn new() -> Self {
        Self
    }

    pub fn initialize(&self) -> Result<(), PlatformError> {
        log::info!("Initializing macOS platform");
        unsafe {
            let app = cocoa::appkit::NSApp();
            if app.is_null() {
                return Err(PlatformError::InitializationFailed("Failed to get NSApp".to_string()));
            }
        }
        Ok(())
    }

    pub fn create_window(&self, width: u32, height: u32, title: &str) -> Result<MacOSWindow, PlatformError> {
        MacOSWindow::new(width, height, title)
    }

    pub fn get_primary_monitor(&self) -> Result<MonitorInfo, PlatformError> {
        unsafe {
            let display_id = CGMainDisplayID();
            let width = core_graphics::display::CGDisplayPixelsWide(display_id) as u32;
            let height = core_graphics::display::CGDisplayPixelsHigh(display_id) as u32;
            let refresh_rate = 60; // Default, actual refresh rate detection is more complex
            
            Ok(MonitorInfo {
                width,
                height,
                refresh_rate,
                scale_factor: 1.0,
            })
        }
    }

    pub fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        let mut displays = Vec::new();
        
        unsafe {
            let max_displays = 32;
            let mut display_ids = vec![0u32; max_displays];
            let mut display_count = 0u32;
            
            let result = core_graphics::display::CGGetActiveDisplayList(
                max_displays as u32,
                display_ids.as_mut_ptr(),
                &mut display_count,
            );
            
            if result == 0 {
                for i in 0..display_count {
                    let display_id = display_ids[i as usize];
                    let width = core_graphics::display::CGDisplayPixelsWide(display_id) as u32;
                    let height = core_graphics::display::CGDisplayPixelsHigh(display_id) as u32;
                    let bounds = core_graphics::display::CGDisplayBounds(display_id);
                    
                    displays.push(DisplayInfo {
                        id: display_id,
                        name: format!("Display {}", i),
                        width,
                        height,
                        x: bounds.origin.x as i32,
                        y: bounds.origin.y as i32,
                        is_primary: display_id == CGMainDisplayID(),
                    });
                }
            }
        }
        
        Ok(displays)
    }
}

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub width: u32,
    pub height: u32,
    pub refresh_rate: u32,
    pub scale_factor: f32,
}

#[derive(Debug, Clone)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("Window creation failed: {0}")]
    WindowCreationFailed(String),
    #[error("Display enumeration failed")]
    DisplayEnumerationFailed,
    #[error("Platform initialization failed: {0}")]
    InitializationFailed(String),
}

impl Default for MacOSPlatform {
    fn default() -> Self {
        Self::new()
    }
}