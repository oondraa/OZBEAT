//! The main window has no frame, but winit still keeps a 1 px non-client
//! strip at its top (for the drop shadow). In fullscreen Windows paints that
//! strip as a white line whenever the window loses focus, e.g. to the second
//! screen's window (Ctrl+D). This makes the whole window client area while it
//! covers its monitor; otherwise winit handles it as before.

use raw_window_handle::HasWindowHandle;

#[cfg(windows)]
pub fn install(window: &impl HasWindowHandle) {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;

    let Ok(handle) = window.window_handle() else {
        return;
    };
    if let RawWindowHandle::Win32(handle) = handle.as_raw() {
        // SAFETY: the handle is the live main window, which outlives the subclass.
        unsafe {
            SetWindowSubclass(handle.hwnd.get() as _, Some(proc), 1, 0);
        }
    }
}

#[cfg(not(windows))]
pub fn install(_window: &impl HasWindowHandle) {}

#[cfg(windows)]
unsafe extern "system" fn proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: usize,
    lparam: isize,
    _id: usize,
    _data: usize,
) -> isize {
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromRect,
    };
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{NCCALCSIZE_PARAMS, WM_NCCALCSIZE};

    if msg == WM_NCCALCSIZE && wparam != 0 {
        // SAFETY: with a nonzero wparam, lparam points to NCCALCSIZE_PARAMS.
        let rect = unsafe { (*(lparam as *const NCCALCSIZE_PARAMS)).rgrc[0] };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..unsafe { std::mem::zeroed() }
        };
        let monitor = unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) };
        let covers = !monitor.is_null()
            && unsafe { GetMonitorInfoW(monitor, &mut info) } != 0
            && rect.left <= info.rcMonitor.left
            && rect.top <= info.rcMonitor.top
            && rect.right >= info.rcMonitor.right
            && rect.bottom >= info.rcMonitor.bottom;
        if covers {
            // Leaving the proposed rect as is: the client area is the window.
            return 0;
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
