//! Platform abstraction layer for cross-platform windowing and display management.
//!
//! This module provides a unified interface for creating windows and managing displays
//! across different operating systems (Windows, macOS, and Linux).

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

// Re-export common types that are shared across all platforms
pub use self::platform_impl::*;

// Platform-specific type aliases for cleaner API
#[cfg(target_os = "windows")]
mod platform_impl {
    pub use super::windows::{WindowsPlatform as Platform, WindowsWindow as Window};
}

#[cfg(target_os = "macos")]
mod platform_impl {
    pub use super::macos::{MacOSPlatform as Platform, MacOSWindow as Window};
}

#[cfg(target_os = "linux")]
mod platform_impl {
    pub use super::linux::{LinuxPlatform as Platform, LinuxWindow as Window};
}

/// Cross-platform window and display management functionality.
///
/// This trait defines the common interface that all platform implementations must provide.
pub trait PlatformInterface {
    type Window;
    type Error;

    /// Initialize the platform.
    fn initialize(&self) -> Result<(), Self::Error>;
    
    /// Create a new window with the specified dimensions and title.
    fn create_window(&self, width: u32, height: u32, title: &str) -> Result<Self::Window, Self::Error>;
    
    /// Get information about the primary monitor.
    fn get_primary_monitor(&self) -> Result<MonitorInfo, Self::Error>;
    
    /// Enumerate all available displays.
    fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, Self::Error>;
}

// Implement the trait for each platform
#[cfg(target_os = "windows")]
impl PlatformInterface for WindowsPlatform {
    type Window = WindowsWindow;
    type Error = PlatformError;

    fn initialize(&self) -> Result<(), Self::Error> {
        WindowsPlatform::initialize(self)
    }

    fn create_window(&self, width: u32, height: u32, title: &str) -> Result<Self::Window, Self::Error> {
        WindowsPlatform::create_window(self, width, height, title)
    }

    fn get_primary_monitor(&self) -> Result<MonitorInfo, Self::Error> {
        WindowsPlatform::get_primary_monitor(self)
    }

    fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, Self::Error> {
        WindowsPlatform::enumerate_displays(self)
    }
}

#[cfg(target_os = "macos")]
impl PlatformInterface for MacOSPlatform {
    type Window = MacOSWindow;
    type Error = PlatformError;

    fn initialize(&self) -> Result<(), Self::Error> {
        MacOSPlatform::initialize(self)
    }

    fn create_window(&self, width: u32, height: u32, title: &str) -> Result<Self::Window, Self::Error> {
        MacOSPlatform::create_window(self, width, height, title)
    }

    fn get_primary_monitor(&self) -> Result<MonitorInfo, Self::Error> {
        MacOSPlatform::get_primary_monitor(self)
    }

    fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, Self::Error> {
        MacOSPlatform::enumerate_displays(self)
    }
}

#[cfg(target_os = "linux")]
impl PlatformInterface for LinuxPlatform {
    type Window = LinuxWindow;
    type Error = PlatformError;

    fn initialize(&self) -> Result<(), Self::Error> {
        LinuxPlatform::initialize(self)
    }

    fn create_window(&self, width: u32, height: u32, title: &str) -> Result<Self::Window, Self::Error> {
        LinuxPlatform::create_window(self, width, height, title)
    }

    fn get_primary_monitor(&self) -> Result<MonitorInfo, Self::Error> {
        LinuxPlatform::get_primary_monitor(self)
    }

    fn enumerate_displays(&self) -> Result<Vec<DisplayInfo>, Self::Error> {
        LinuxPlatform::enumerate_displays(self)
    }
}

/// Convenience function to create a platform instance.
///
/// Returns the appropriate platform implementation for the current operating system.
pub fn create_platform() -> Platform {
    Platform::new()
}

/// Convenience function to initialize the platform and create a window.
///
/// This is a high-level function that handles platform initialization and window creation
/// in a single call for simple use cases.
pub fn create_platform_and_window(
    width: u32,
    height: u32,
    title: &str,
) -> Result<(Platform, Window), PlatformError> {
    let platform = create_platform();
    platform.initialize()?;
    let window = platform.create_window(width, height, title)?;
    Ok((platform, window))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_creation() {
        let platform = create_platform();
        assert!(platform.initialize().is_ok());
    }

    #[test]
    fn test_monitor_info() {
        let platform = create_platform();
        platform.initialize().unwrap();
        let monitor_info = platform.get_primary_monitor().unwrap();
        assert!(monitor_info.width > 0);
        assert!(monitor_info.height > 0);
    }

    #[test]
    fn test_display_enumeration() {
        let platform = create_platform();
        platform.initialize().unwrap();
        let displays = platform.enumerate_displays().unwrap();
        assert!(!displays.is_empty());
        assert!(displays.iter().any(|d| d.is_primary));
    }
}