//! Global cursor position and active-window geometry.
//!
//! Unlike Wayland, Windows hands these out directly: `GetCursorPos` plus the
//! foreground window's `GetWindowRect`.

use crate::platform::{CursorInfo, CursorTracker};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, GetWindowRect};

#[derive(Default)]
pub struct WinCursor;

impl WinCursor {
    pub fn new() -> Self {
        WinCursor
    }
}

impl CursorTracker for WinCursor {
    fn fetch(&self) -> Option<CursorInfo> {
        unsafe {
            let mut p = POINT::default();
            GetCursorPos(&mut p).ok()?;

            let hwnd = GetForegroundWindow();
            let (fx, fy, fw, fh) = if !hwnd.is_invalid() {
                let mut r = RECT::default();
                if GetWindowRect(hwnd, &mut r).is_ok() {
                    (r.left, r.top, r.right - r.left, r.bottom - r.top)
                } else {
                    (0, 0, 0, 0)
                }
            } else {
                (0, 0, 0, 0)
            };

            Some(CursorInfo {
                x: p.x,
                y: p.y,
                frame_x: fx,
                frame_y: fy,
                frame_w: fw,
                frame_h: fh,
            })
        }
    }
}
