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
    /// Pointer movement (evdev relative units) accumulated while the button was
    /// held down — used to tell a drag from a plain click.
    pub drag: (i32, i32),
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
    /// Owning application / process name (KWin `resourceClass`), if available.
    pub process: Option<String>,
    /// Description of the clicked UI element (AT-SPI), if available.
    pub element: Option<String>,
    /// Bounding box `[x, y, w, h]` of the clicked element in **image-pixel**
    /// coordinates (already mapped from AT-SPI screen coords through the
    /// capture's frame offset + scale). Enables one-click redaction of the
    /// clicked element in the editor. `None` for screen captures or when no box
    /// was resolved.
    pub element_box: Option<[u32; 4]>,
    /// Editor-supplied replacement for the display text. When set, it wins over
    /// the auto-generated description; the auto text stays available via
    /// [`Step::auto_describe`] so an override can be reverted.
    pub description_override: Option<String>,
    /// The screenshot shows the whole screen, not a single window (panel,
    /// desktop or popup-menu click).
    pub is_screen: bool,
    /// Two rapid clicks of the same button merged into a single double-click
    /// step (see `capture.double_click_ms`).
    pub double: bool,
    /// The button was pressed, dragged, and released elsewhere (see
    /// `capture.drag_min_px`) — a drag-and-drop step.
    pub drag: bool,
}

impl Step {
    /// The action verb for this step: the per-button label, or the double-click
    /// label when two rapid clicks were merged.
    fn action_label(&self) -> &'static str {
        let t = crate::i18n::tr();
        if self.drag {
            t.click_drag
        } else if self.double {
            t.click_double
        } else {
            self.button.label()
        }
    }

    /// The effective one-line description: an editor override if present,
    /// otherwise the auto-generated text.
    pub fn describe(&self) -> String {
        match &self.description_override {
            Some(s) if !s.is_empty() => s.clone(),
            _ => self.auto_describe(),
        }
    }

    /// The auto-generated one-line description (localized), ignoring any
    /// override — the source text kept in `session.json` for revert.
    pub fn auto_describe(&self) -> String {
        let t = crate::i18n::tr();
        let verb = self.action_label();
        let action = match &self.element {
            Some(el) if !el.is_empty() => t
                .action_on
                .replace("{action}", verb)
                .replace("{element}", el),
            _ => verb.to_string(),
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
            process: None,
            element: element.map(String::from),
            element_box: None,
            description_override: None,
            is_screen: false,
            double: false,
            drag: false,
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
    fn describe_double_click() {
        let mut s = step(Some("Editor"), Some("button “Save”"));
        s.double = true;
        assert_eq!(
            s.describe(),
            "Double click on button “Save” in window “Editor”"
        );
    }

    #[test]
    fn override_wins_over_auto_but_keeps_it() {
        let mut s = step(Some("Editor"), Some("button “Save”"));
        s.description_override = Some("Click Save to store the file".into());
        assert_eq!(s.describe(), "Click Save to store the file");
        // The auto text is still available for revert.
        assert_eq!(
            s.auto_describe(),
            "Left click on button “Save” in window “Editor”"
        );
        // An empty override falls back to auto.
        s.description_override = Some(String::new());
        assert_eq!(s.describe(), s.auto_describe());
    }

    #[test]
    fn describe_drag() {
        let mut s = step(Some("Files"), Some("list item “report.pdf”"));
        s.drag = true;
        assert_eq!(
            s.describe(),
            "Drag and drop on list item “report.pdf” in window “Files”"
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
