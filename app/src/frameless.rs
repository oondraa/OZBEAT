//! The main window has no frame, but winit still keeps a 1 px non-client
//! strip at its top (for the drop shadow). In fullscreen Windows paints that
//! strip as a white line whenever the window loses focus, e.g. to the second
//! screen's window (Ctrl+D). This makes the whole window client area while it
//! covers its monitor; otherwise winit handles it as before.

use raw_window_handle::HasWindowHandle;

/// The main window, once `install` has seen it.
#[cfg(windows)]
static MAIN: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
/// The window is the lyrics over the desktop (see `set_glass`): no frame.
#[cfg(windows)]
static BARE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// What Windows would draw under glass: the frame and its buttons.
#[cfg(windows)]
const FRAME: u32 = {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
    };
    WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX
};

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

/// The main window's handle, null before `install`.
#[cfg(windows)]
pub fn main_window() -> windows_sys::Win32::Foundation::HWND {
    MAIN.load(std::sync::atomic::Ordering::Relaxed) as _
}

/// See-through (`on`) or not: with the glass over the whole window, Windows
/// shows what lies behind wherever the picture is clear, instead of filling
/// it with white.
#[cfg(windows)]
pub fn set_glass(on: bool) {
    use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
    use windows_sys::Win32::UI::Controls::MARGINS;

    BARE.store(on, std::sync::atomic::Ordering::Relaxed);
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

/// Keeps the window bare while it is the lyrics over the desktop; call every
/// frame then, since winit sets the styles again whenever it changes one.
///
/// Under glass Windows draws the frame and the close, maximize and minimize
/// buttons of any window that has them, so they go. And letting the mouse
/// through makes winit turn the window "layered", which shows nothing of its
/// picture until it is given an opacity. Leaving the overlay, winit's own
/// style changes put the frame back.
#[cfg(windows)]
pub fn keep_bare() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW, LWA_ALPHA, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetLayeredWindowAttributes, SetWindowLongPtrW,
        SetWindowPos, WS_EX_LAYERED,
    };

    let hwnd = MAIN.load(std::sync::atomic::Ordering::Relaxed) as windows_sys::Win32::Foundation::HWND;
    if hwnd.is_null() {
        return;
    }
    // SAFETY: a live window handle; the calls only read or set its styles.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        if style & FRAME != 0 {
            SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !FRAME) as _);
            let flags = SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
            SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, flags);
        }
        if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_LAYERED != 0 {
            SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        }
    }
}

#[cfg(not(windows))]
pub fn keep_bare() {}

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
        GWL_STYLE, GetClientRect, NCCALCSIZE_PARAMS, STYLESTRUCT, WM_ERASEBKGND, WM_NCACTIVATE,
        WM_NCCALCSIZE, WM_NCPAINT, WM_STYLECHANGING,
    };

    if BARE.load(std::sync::atomic::Ordering::Relaxed) {
        match msg {
            // winit puts the frame back on every focus change; keep it off.
            WM_STYLECHANGING if wparam as i32 == GWL_STYLE => {
                // SAFETY: for WM_STYLECHANGING, lparam points to the new styles.
                unsafe { (*(lparam as *mut STYLESTRUCT)).styleNew &= !FRAME };
                return 0;
            }
            // A click would light up the title bar: nothing to paint there.
            WM_NCPAINT => return 0,
            // -1: change the active state without redrawing the frame.
            WM_NCACTIVATE => return unsafe { DefSubclassProc(hwnd, msg, wparam, -1) },
            _ => {}
        }
    }

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
