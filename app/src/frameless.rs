//! The main window has no frame, but winit still keeps a 1 px non-client
//! strip at its top (for the drop shadow). In fullscreen Windows paints that
//! strip as a white line whenever the window loses focus, e.g. to the second
//! screen's window (Ctrl+D). This makes the whole window client area while it
//! covers its monitor; otherwise winit handles it as before.

use raw_window_handle::HasWindowHandle;

/// The main window, once `install` has seen it.
#[cfg(windows)]
static MAIN: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

#[cfg(windows)]
pub fn install(window: &impl HasWindowHandle) {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;

    let Ok(handle) = window.window_handle() else {
        return;
    };
    if let RawWindowHandle::Win32(handle) = handle.as_raw() {
        MAIN.store(handle.hwnd.get(), std::sync::atomic::Ordering::Relaxed);
        // SAFETY: the handle is the live main window, which outlives the subclass.
        unsafe {
            SetWindowSubclass(handle.hwnd.get() as _, Some(proc), 1, 0);
        }
    }
}

#[cfg(not(windows))]
pub fn install(_window: &impl HasWindowHandle) {}

/// See-through (`on`) or not: with the glass over the whole window, Windows
/// shows what lies behind wherever the picture is clear, instead of filling
/// it with white.
#[cfg(windows)]
pub fn set_glass(on: bool) {
    use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
    use windows_sys::Win32::UI::Controls::MARGINS;

    let hwnd = MAIN.load(std::sync::atomic::Ordering::Relaxed) as windows_sys::Win32::Foundation::HWND;
    if hwnd.is_null() {
        return;
    }
    // -1 on every side: the whole window is glass.
    let m = if on { -1 } else { 0 };
    let margins = MARGINS {
        cxLeftWidth: m,
        cxRightWidth: m,
        cyTopHeight: m,
        cyBottomHeight: m,
    };
    // SAFETY: a live window handle and a valid MARGINS.
    unsafe {
        DwmExtendFrameIntoClientArea(hwnd, &margins);
        // Paint what lies under the picture black, which glass shows as clear.
        use windows_sys::Win32::Graphics::Gdi::{RDW_ERASE, RDW_ERASENOW, RDW_INVALIDATE, RedrawWindow};
        RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_ERASE | RDW_INVALIDATE | RDW_ERASENOW);
    }
}

#[cfg(not(windows))]
pub fn set_glass(_on: bool) {}

/// Letting the mouse through makes winit turn the window "layered", and a
/// layered window shows nothing of its picture until it is given an opacity.
/// Call while the mouse passes through; harmless before winit gets to it.
#[cfg(windows)]
pub fn show_layered() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, WS_EX_LAYERED,
    };

    let hwnd = MAIN.load(std::sync::atomic::Ordering::Relaxed) as windows_sys::Win32::Foundation::HWND;
    if hwnd.is_null() {
        return;
    }
    // SAFETY: a live window handle; both calls only read or set its styles.
    unsafe {
        if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_LAYERED != 0 {
            SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        }
    }
}

#[cfg(not(windows))]
pub fn show_layered() {}

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
    use windows_sys::Win32::Graphics::Gdi::{BLACK_BRUSH, FillRect, GetStockObject};
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, NCCALCSIZE_PARAMS, WM_ERASEBKGND, WM_NCCALCSIZE,
    };

    if msg == WM_ERASEBKGND {
        // Black under the picture: hidden by it normally, clear under glass.
        let mut client = unsafe { std::mem::zeroed() };
        unsafe {
            GetClientRect(hwnd, &mut client);
            FillRect(wparam as _, &client, GetStockObject(BLACK_BRUSH) as _);
        }
        return 1;
    }

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
