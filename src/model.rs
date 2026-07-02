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
    #[cfg(target_os = "linux")]
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
    /// The screenshot shows the whole screen, not a single window (panel,
    /// desktop or popup-menu click).
    pub is_screen: bool,
}

impl Step {
    /// One-line description of what happened in this step (localized).
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
            _ if self.is_screen => t.in_screen.replace("{action}", &action),
            _ => t.in_active_window.replace("{action}", &action),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn button_from_evdev_codes() {
        assert_eq!(Button::from_evdev_code(272), Some(Button::Left));
        assert_eq!(Button::from_evdev_code(273), Some(Button::Right));
        assert_eq!(Button::from_evdev_code(274), Some(Button::Middle));
        assert_eq!(Button::from_evdev_code(275), None); // BTN_SIDE
        assert_eq!(Button::from_evdev_code(28), None); // KEY_ENTER
    }

    fn step(window_title: Option<&str>, element: Option<&str>) -> Step {
        Step {
            index: 1,
            button: Button::Left,
            time: "12:00:00".into(),
            image_file: "step-001.png".into(),
            window_title: window_title.map(String::from),
            element: element.map(String::from),
            is_screen: false,
        }
    }

    /// `tr()` falls back to English in tests (i18n::init only runs in main),
    /// so these assertions are deterministic.
    #[test]
    fn describe_combines_element_and_window() {
        assert_eq!(
            step(Some("Editor"), Some("button “Save”")).describe(),
            "Left click on button “Save” in window “Editor”"
        );
        assert_eq!(
            step(None, None).describe(),
            "Left click in the active window"
        );
        // Empty strings count as absent, not as empty labels.
        assert_eq!(
            step(Some(""), Some("")).describe(),
            "Left click in the active window"
        );
    }

    #[test]
    fn describe_full_screen_capture() {
        let mut s = step(None, Some("button “Kickoff”"));
        s.is_screen = true;
        assert_eq!(
            s.describe(),
            "Left click on button “Kickoff” (full-screen view)"
        );
    }
}
