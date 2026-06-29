//! Global click capture via a low-level mouse hook (`WH_MOUSE_LL`).
//!
//! Low-level hooks require a message loop on the installing thread, so the hook
//! runs on its own thread; button-down events are forwarded over a channel. The
//! hook procedure is a bare `extern "system"` function with no user data, so the
//! sender lives in a process-global `OnceLock`.

use crate::model::{Button, Click};
use crate::platform::ClickSource;
use anyhow::{Context, Result};
use std::sync::OnceLock;
use std::sync::mpsc::Sender;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, MSG, SetWindowsHookExW, WH_MOUSE_LL, WM_LBUTTONDOWN,
    WM_MBUTTONDOWN, WM_RBUTTONDOWN,
};

static SENDER: OnceLock<Sender<Click>> = OnceLock::new();

/// Low-level mouse-hook click source for Windows.
#[derive(Default)]
pub struct WindowsClickSource;

impl ClickSource for WindowsClickSource {
    /// Installs the hook on a dedicated thread.
    fn start(&self, tx: Sender<Click>) -> Result<()> {
        let _ = SENDER.set(tx);
        std::thread::Builder::new()
            .name("win-mouse-hook".into())
            .spawn(|| unsafe {
                let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), None, 0) {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("[stepshot] SetWindowsHookExW failed: {e}");
                        return;
                    }
                };
                // Pump messages so the hook keeps firing; it has nothing to dispatch.
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
                let _ = hook; // kept alive for the lifetime of the loop
            })
            .context("could not start the mouse-hook thread")?;
        Ok(())
    }
}

unsafe extern "system" fn hook_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode == HC_ACTION as i32 {
        let button = match wparam.0 as u32 {
            WM_LBUTTONDOWN => Some(Button::Left),
            WM_RBUTTONDOWN => Some(Button::Right),
            WM_MBUTTONDOWN => Some(Button::Middle),
            _ => None,
        };
        if let (Some(button), Some(tx)) = (button, SENDER.get()) {
            let _ = tx.send(Click { button });
        }
    }
    unsafe { CallNextHookEx(None, ncode, wparam, lparam) }
}
