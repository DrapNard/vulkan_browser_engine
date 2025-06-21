use super::{PlatformError, MonitorInfo};
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle, Win32WindowHandle};
use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use winapi::shared::minwindef::*;
use winapi::shared::windef::*;
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::winuser::*;

pub struct WindowsWindow {
    hwnd: HWND,
    width: u32,
    height: u32,
    title: String,
}

impl WindowsWindow {
    pub fn new(width: u32, height: u32, title: &str) -> Result<Self, PlatformError> {
        unsafe {
            let class_name: Vec<u16> = OsStr::new("VulkanRendererWindow")
                .encode_wide()
                .chain(once(0))
                .collect();

            let h_instance = GetModuleHandleW(ptr::null());

            let wnd_class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_OWNDC | CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: h_instance,
                hIcon: ptr::null_mut(),
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                hbrBackground: ptr::null_mut(),
                lpszMenuName: ptr::null(),
                lpszClassName: class_name.as_ptr(),
                hIconSm: ptr::null_mut(),
            };

            RegisterClassExW(&wnd_class);

            let window_title: Vec<u16> = OsStr::new(title)
                .encode_wide()
                .chain(once(0))
                .collect();

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                window_title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                width as i32,
                height as i32,
                ptr::null_mut(),
                ptr::null_mut(),
                h_instance,
                ptr::null_mut(),
            );

            if hwnd.is_null() {
                return Err(PlatformError::WindowCreationFailed("CreateWindowExW failed".to_string()));
            }

            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);

            Ok(Self {
                hwnd,
                width,
                height,
                title: title.to_string(),
            })
        }
    }

    pub fn poll_events(&self) -> Vec<WindowEvent> {
        let mut events = Vec::new();
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, self.hwnd, 0, 0, PM_REMOVE) != 0 {
                match msg.message {
                    WM_PAINT => {
                        events.push(WindowEvent::Redraw);
                    }
                    WM_KEYDOWN => {
                        events.push(WindowEvent::KeyPressed {
                            keycode: msg.wParam as u32,
                            modifiers: 0,
                        });
                    }
                    WM_LBUTTONDOWN => {
                        let x = (msg.lParam & 0xFFFF) as i16 as f64;
                        let y = ((msg.lParam >> 16) & 0xFFFF) as i16 as f64;
                        events.push(WindowEvent::MousePressed { x, y, button: 1 });
                    }
                    WM_SIZE => {
                        let width = (msg.lParam & 0xFFFF) as u32;
                        let height = ((msg.lParam >> 16) & 0xFFFF) as u32;
                        events.push(WindowEvent::Resized { width, height });
                    }
                    WM_CLOSE => {
                        events.push(WindowEvent::Closed);
                    }
                    _ => {}
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        events
    }

    pub fn get_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        unsafe {
            SetWindowPos(
                self.hwnd,
                ptr::null_mut(),
                0, 0,
                width as i32,
                height as i32,
                SWP_NOMOVE | SWP_NOZORDER,
            );
        }
        self.width = width;
        self.height = height;
    }

    pub fn set_title(&mut self, title: &str) {
        let window_title: Vec<u16> = OsStr::new(title)
            .encode_wide()
            .chain(once(0))
            .collect();
        unsafe {
            SetWindowTextW(self.hwnd, window_title.as_ptr());
        }
        self.title = title.to_string();
    }

    pub fn show(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_SHOW);
        }
    }

    pub fn hide(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn get_vulkan_surface_extensions(&self) -> Vec<&'static str> {
        vec!["VK_KHR_surface", "VK_KHR_win32_surface"]
    }
}

unsafe impl HasRawWindowHandle for WindowsWindow {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let mut handle = Win32WindowHandle::empty();
        handle.hwnd = self.hwnd as *mut std::ffi::c_void;
        handle.hinstance = GetModuleHandleW(ptr::null_mut()) as *mut std::ffi::c_void;
        RawWindowHandle::Win32(handle)
    }
}

impl Drop for WindowsWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
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