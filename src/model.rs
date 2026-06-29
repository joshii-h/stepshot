//! Platform-neutral data types for a recorded session.

use std::fmt;

/// Which mouse button triggered the step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    /// evdev codes: BTN_LEFT=272, BTN_RIGHT=273, BTN_MIDDLE=274.
    pub fn from_evdev_code(code: u16) -> Option<Self> {
        match code {
            272 => Some(Button::Left),
            273 => Some(Button::Right),
            274 => Some(Button::Middle),
            _ => None,
        }
    }

    /// Human-readable label used in the description.
    pub fn label(self) -> &'static str {
        let t = crate::i18n::tr();
        match self {
            Button::Left => t.click_left,
            Button::Right => t.click_right,
            Button::Middle => t.click_middle,
        }
    }
}

impl fmt::Display for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A click reported by the input backend (no screenshot yet).
#[derive(Debug, Clone, Copy)]
pub struct Click {
    pub button: Button,
}

/// A fully captured step: click + screenshot + context.
#[derive(Debug, Clone)]
pub struct Step {
    /// 1-based step number within the session.
    pub index: usize,
    pub button: Button,
    /// Capture timestamp, preformatted (HH:MM:SS).
    pub time: String,
    /// File name (relative to the session folder) of the screenshot.
    pub image_file: String,
    /// Window title, if the capture backend could resolve it.
    pub window_title: Option<String>,
    /// Description of the clicked UI element (AT-SPI), if available.
    pub element: Option<String>,
}

impl Step {
    /// One-line description of what happened in this step.
    pub fn describe(&self) -> String {
        let t = crate::i18n::tr();
        let action = match &self.element {
            Some(el) if !el.is_empty() => t
                .action_on
                .replace("{action}", self.button.label())
                .replace("{element}", el),
            _ => self.button.label().to_string(),
        };
        match &self.window_title {
            Some(title) if !title.is_empty() => t
                .in_window
                .replace("{action}", &action)
                .replace("{title}", title),
            _ => t.in_active_window.replace("{action}", &action),
        }
    }
}
