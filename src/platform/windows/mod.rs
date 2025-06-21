pub mod window;

pub use window::*;

use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use winapi::um::winuser::*;

pub struct WindowsPlatform;

impl WindowsPlatform {
    pub fn new() -> Self {
        Self
    }

    pub fn initialize(&self) -> Result<(), PlatformError> {
        log::info!("Initializing Windows platform");
        Ok(())
    }

    pub fn create_window(&self, width: u32, height: u32, title: &str) -> Result<WindowsWindow, PlatformError> {
        WindowsWindow::new(width, height, title)
    }

    pub fn get_primary_monitor(&self) -> Result<MonitorInfo, PlatformError> {
        unsafe {
            let width = GetSystemMetrics(SM_CXSCREEN) as u32;
            let height = GetSystemMetrics(SM_CYSCREEN) as u32;
            
            Ok(MonitorInfo {
                width,
                height,
                refresh_rate: 60,
                scale_factor: 1.0,
            })
        }
    }

    pub fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        let mut displays = Vec::new();
        
        unsafe {
            let mut device_num = 0;
            let mut display_device: DISPLAY_DEVICEW = std::mem::zeroed();
            display_device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;

            while EnumDisplayDevicesW(std::ptr::null(), device_num, &mut display_device, 0) != 0 {
                if display_device.StateFlags & DISPLAY_DEVICE_ACTIVE != 0 {
                    let mut dev_mode: DEVMODEW = std::mem::zeroed();
                    dev_mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
                    
                    if EnumDisplaySettingsW(display_device.DeviceName.as_ptr(), ENUM_CURRENT_SETTINGS, &mut dev_mode) != 0 {
                        displays.push(DisplayInfo {
                            id: device_num,
                            name: "Windows Display".to_string(),
                            width: dev_mode.dmPelsWidth,
                            height: dev_mode.dmPelsHeight,
                            x: dev_mode.dmPosition.x,
                            y: dev_mode.dmPosition.y,
                            is_primary: display_device.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0,
                        });
                    }
                }
                device_num += 1;
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

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}