//! Platform abstraction.
//!
//! The OS-specific work — capturing clicks, screenshotting the active window,
//! finding the cursor, naming the element under it — sits behind these traits.
//! Each OS provides one backend; the rest of the program (`model`, `report`,
//! `annotate`, the session logic in `session`) is platform-neutral and talks
//! only to the traits and the shared data types defined here.
//!
//! - Linux/KDE: `capture`, `cursor`, `a11y`, `input` (evdev + KWin + AT-SPI).
//! - Windows: the `win` module (`WH_MOUSE_LL` + `PrintWindow` + UI Automation).

use crate::model::Click;
use anyhow::Result;
use image::RgbaImage;
use std::sync::mpsc::Sender;

/// A captured image plus optional context.
pub struct Capture {
    pub image: RgbaImage,
    pub window_title: Option<String>,
    /// Scale factor (HiDPI): image pixels = logical coords * scale.
    pub scale: f64,
    /// True when this is a full-screen capture instead of a window capture —
    /// used for clicks the active window wouldn't show (panel, desktop, popup
    /// menus) and as fallback when the active "window" had no visible content.
    /// The session then skips the window-relative click marker; the backend is
    /// responsible for making the click location visible in the image (baked-in
    /// cursor or a drawn marker).
    pub is_screen: bool,
}

/// Global cursor position and frame rect of the active window (screen coords),
/// plus platform hints about what the cursor is over.
#[derive(Debug, Clone, Default)]
pub struct CursorInfo {
    pub x: i32,
    pub y: i32,
    pub frame_x: i32,
    pub frame_y: i32,
    pub frame_w: i32,
    pub frame_h: i32,
    /// The cursor is over a popup surface (context menu, dropdown — not a
    /// tooltip). Popups are separate surfaces and never show up in a window
    /// capture of their parent, so such clicks need a screen capture.
    pub in_popup: bool,
    /// Output (monitor) name under the cursor, when the platform knows it
    /// (KWin: e.g. `DP-5`); empty if unknown or not applicable. Only the KDE
    /// backend reads it back, hence the target-specific allow.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub screen: String,
}

impl CursorInfo {
    /// Is the cursor inside the active window's frame rect? False also when
    /// there is no active window (frame is 0×0) — e.g. a click on the desktop
    /// or the panel, which never becomes the "active window".
    pub fn in_active_window(&self) -> bool {
        self.frame_w > 0
            && self.frame_h > 0
            && self.x >= self.frame_x
            && self.x < self.frame_x + self.frame_w
            && self.y >= self.frame_y
            && self.y < self.frame_y + self.frame_h
    }
}

/// A detected UI element (name + role), e.g. button “Save”.
#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    pub role: String,
}

impl Element {
    /// Description like “button ‘Save’” or just “text field”.
    pub fn describe(&self) -> String {
        match (self.role.trim(), self.name.trim()) {
            (r, n) if !r.is_empty() && !n.is_empty() => format!("{r} “{n}”"),
            (r, _) if !r.is_empty() => r.to_string(),
            (_, n) if !n.is_empty() => format!("“{n}”"),
            _ => crate::i18n::tr().element_generic.to_string(),
        }
    }
}

/// Heuristic: is the image essentially one uniform color? Such captures come
/// from windows with no visible content — KWin's transparent Xwayland video
/// bridge, or the bare desktop. Real windows include decorations (titlebar,
/// borders), so they never read as uniform. Sampled on a grid so it stays
/// cheap on multi-megapixel images.
pub fn is_blank(img: &RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return true;
    }
    let reference = *img.get_pixel(0, 0);
    let step_x = (w / 64).max(1);
    let step_y = (h / 64).max(1);
    let (mut total, mut same) = (0u32, 0u32);
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            total += 1;
            if *img.get_pixel(x, y) == reference {
                same += 1;
            }
            x += step_x;
        }
        y += step_y;
    }
    same as f32 / total as f32 >= 0.995
}

/// A source of global clicks. Reports every button press over the channel.
pub trait ClickSource {
    /// Starts capturing. Clicks are delivered asynchronously over `tx`.
    fn start(&self, tx: Sender<Click>) -> Result<()>;
}

/// Backend that photographs the currently active window or the screen.
pub trait WindowCapturer {
    fn capture_active_window(&self) -> Result<Capture>;

    /// Capture the monitor/screen under the cursor — used when the click
    /// doesn't land in the active window (panel, desktop) or hits a popup.
    /// The returned image must make the click location visible (bake the real
    /// cursor in, or draw a marker at `ci`'s position). Backends without a
    /// screen path may return `Err`; the session then falls back to the
    /// window capture.
    fn capture_screen_under_cursor(&self, ci: Option<&CursorInfo>) -> Result<Capture>;
}

/// Backend that reports the global cursor position and active-window geometry.
pub trait CursorTracker {
    fn fetch(&self) -> Option<CursorInfo>;
}

/// Backend that names the UI element at a screen coordinate.
///
/// Enabling/restoring the platform's accessibility layer around a recording
/// session stays on the concrete backend types (`Atspi::enable`/`restore` on
/// Linux; no-ops on Windows, where UI Automation is always available) — the
/// per-OS run loops own those types anyway.
pub trait ElementResolver {
    fn element_at(&self, x: i32, y: i32) -> Option<Element>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_detection() {
        let uniform = RgbaImage::from_pixel(200, 100, image::Rgba([37, 37, 37, 255]));
        assert!(is_blank(&uniform));

        let mut real = uniform.clone();
        for y in 0..50 {
            for x in 0..100 {
                real.put_pixel(x, y, image::Rgba([200, 10, 10, 255]));
            }
        }
        assert!(!is_blank(&real));

        assert!(is_blank(&RgbaImage::new(0, 0)));
    }

    #[test]
    fn cursor_in_active_window() {
        let ci = CursorInfo {
            x: 50,
            y: 50,
            frame_x: 0,
            frame_y: 0,
            frame_w: 100,
            frame_h: 100,
            ..Default::default()
        };
        assert!(ci.in_active_window());
        let outside = CursorInfo {
            x: 150,
            ..ci.clone()
        };
        assert!(!outside.in_active_window());
        // No active window (0×0 frame) → never "inside".
        let no_win = CursorInfo {
            frame_w: 0,
            frame_h: 0,
            ..ci
        };
        assert!(!no_win.in_active_window());
    }
}
