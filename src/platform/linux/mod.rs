pub mod window;

pub use window::*;

use std::ffi::CString;
use std::ptr;

pub struct LinuxPlatform;

impl LinuxPlatform {
    pub fn new() -> Self {
        Self
    }

    pub fn initialize(&self) -> Result<(), PlatformError> {
        log::info!("Initializing Linux platform");
        Ok(())
    }

    pub fn create_window(&self, width: u32, height: u32, title: &str) -> Result<LinuxWindow, PlatformError> {
        LinuxWindow::new(width, height, title)
    }

    pub fn get_primary_monitor(&self) -> Result<MonitorInfo, PlatformError> {
        Ok(MonitorInfo {
            width: 1920,
            height: 1080,
            refresh_rate: 60,
            scale_factor: 1.0,
        })
    }

    pub fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        Ok(vec![DisplayInfo {
            id: 0,
            name: "Primary Display".to_string(),
            width: 1920,
            height: 1080,
            x: 0,
            y: 0,
            is_primary: true,
        }])
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

impl Default for LinuxPlatform {
    fn default() -> Self {
        Self::new()
    }
}