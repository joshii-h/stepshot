//! Platform abstraction.
//!
//! The OS-specific work — capturing clicks, screenshotting the active window,
//! finding the cursor, naming the element under it — sits behind these traits.
//! Each OS provides one backend; the rest of the program (`model`, `report`,
//! `annotate`, the session loop) is platform-neutral and talks only to the
//! traits and the shared data types defined here.
//!
//! - Linux/KDE: `capture`, `cursor`, `a11y`, `input` (evdev + KWin + AT-SPI).
//! - Windows: the `win` module (`WH_MOUSE_LL` + `PrintWindow` + UI Automation).

use crate::model::Click;
use anyhow::Result;
use image::RgbaImage;
use std::sync::mpsc::Sender;

/// A captured window image plus optional context.
pub struct Capture {
    pub image: RgbaImage,
    pub window_title: Option<String>,
    /// Scale factor (HiDPI): image pixels = logical coords * scale.
    pub scale: f64,
}

/// Global cursor position and frame rect of the active window (screen coords).
#[derive(Debug, Clone, Copy)]
pub struct CursorInfo {
    pub x: i32,
    pub y: i32,
    pub frame_x: i32,
    pub frame_y: i32,
    pub frame_w: i32,
    pub frame_h: i32,
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

/// A source of global clicks. Reports every button press over the channel.
pub trait ClickSource {
    /// Starts capturing. Clicks are delivered asynchronously over `tx`.
    fn start(&self, tx: Sender<Click>) -> Result<()>;
}

/// Backend that photographs the currently active window.
pub trait WindowCapturer {
    fn capture_active_window(&self) -> Result<Capture>;
}

/// Backend that reports the global cursor position and active-window geometry.
pub trait CursorTracker {
    fn fetch(&self) -> Option<CursorInfo>;
}

/// Backend that names the UI element at a screen coordinate.
///
/// `enable`/`restore` bracket a recording session: on Linux they toggle the
/// AT-SPI bridge; on Windows (UI Automation is always available) they are no-ops.
pub trait ElementResolver {
    fn enable(&mut self);
    fn restore(&self);
    fn element_at(&self, x: i32, y: i32) -> Option<Element>;
}
