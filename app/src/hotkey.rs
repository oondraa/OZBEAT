//! Ctrl+Alt+T from anywhere in Windows turns the lyrics over the desktop on
//! and off, also while OZBEAT is minimized and another app has the keyboard.
//! Windows hands such a hotkey to the thread that registered it, so it gets
//! a thread of its own waiting for it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;

/// Starts listening; the flag goes up on each press, for the UI to take down.
pub fn spawn(ctx: egui::Context) -> Arc<AtomicBool> {
    let pressed = Arc::new(AtomicBool::new(false));
    #[cfg(windows)]
    {
        let flag = Arc::clone(&pressed);
        let spawned = std::thread::Builder::new()
            .name("hotkey".into())
            .spawn(move || listen(&flag, &ctx));
        if let Err(e) = spawned {
            eprintln!("hotkey: {e}");
        }
    }
    #[cfg(not(windows))]
    let _ = ctx;
    pressed
}

#[cfg(windows)]
fn listen(flag: &AtomicBool, ctx: &egui::Context) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

    // SAFETY: no window, so the hotkey's messages come to this thread's queue.
    let registered = unsafe {
        RegisterHotKey(
            std::ptr::null_mut(),
            1,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            u32::from(b'T'),
        )
    };
    if registered == 0 {
        eprintln!("hotkey: Ctrl+Alt+T is taken by another app");
        return;
    }
    // SAFETY: MSG is plain data, filled in by GetMessageW.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        if msg.message == WM_HOTKEY {
            flag.store(true, Ordering::Relaxed);
            ctx.request_repaint();
        }
    }
}
