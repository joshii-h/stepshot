//! Global cursor position and active-window geometry.
//!
//! Unlike Wayland, Windows hands these out directly: `GetCursorPos` plus the
//! foreground window's `GetWindowRect`. As a popup hint we check whether the
//! top-level window under the cursor is the foreground window at all — context
//! menus, dropdowns and the taskbar are separate top-level windows.

use crate::platform::{CursorInfo, CursorTracker};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetCursorPos, GetForegroundWindow, GetWindowRect, WindowFromPoint,
};

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

            // The cursor sits over some other top-level window than the
            // foreground one (context menu, dropdown, taskbar) — a window
            // capture of the foreground window would not show it.
            let in_popup = {
                let under = WindowFromPoint(p);
                if under.is_invalid() || hwnd.is_invalid() {
                    false
                } else {
                    let root = GetAncestor(under, GA_ROOT);
                    !root.is_invalid() && root != hwnd
                }
            };

            Some(CursorInfo {
                x: p.x,
                y: p.y,
                frame_x: fx,
                frame_y: fy,
                frame_w: fw,
                frame_h: fh,
                in_popup,
                screen: String::new(),
            })
        }
    }
}
