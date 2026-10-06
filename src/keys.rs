//! Global keyboard capture (opt-in, `[capture] keyboard` / tray toggle).
//!
//! Privacy is the design constraint: stepshot documents *that* something was
//! typed, never *what*. The reader thread classifies every key press on the
//! spot, and ordinary typing leaves it only as a content-free
//! [`KeyEvent::Text`] tick — no key code, no character. Only presses that make
//! a step on their own (Enter, Tab, Esc, F-keys, and shortcuts with
//! Ctrl/Alt/Super) carry their key code, so the main loop can label them
//! (“Ctrl+S”) with the user's real keyboard layout.
//!
//! Devices are only opened once keyboard capture is first enabled, and while
//! the shared gate is closed (not recording, paused, toggled off) no event
//! leaves the thread at all — only the modifier state is kept current.

use crate::input::watch_devices;
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

// evdev key codes (linux/input-event-codes.h).
pub const KEY_ESC: u16 = 1;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_ENTER: u16 = 28;
const KEY_LEFTCTRL: u16 = 29;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_LEFTALT: u16 = 56;
pub const KEY_SPACE: u16 = 57;
pub const KEY_KPENTER: u16 = 96;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_RIGHTALT: u16 = 100; // AltGr: a text level, not a shortcut modifier
pub const KEY_HOME: u16 = 102;
pub const KEY_UP: u16 = 103;
pub const KEY_PAGEUP: u16 = 104;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_END: u16 = 107;
pub const KEY_DOWN: u16 = 108;
pub const KEY_PAGEDOWN: u16 = 109;
pub const KEY_INSERT: u16 = 110;
pub const KEY_DELETE: u16 = 111;
const KEY_LEFTMETA: u16 = 125;
const KEY_RIGHTMETA: u16 = 126;

/// Modifiers held during a key press (AltGr deliberately excluded).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Mods {
    /// A shortcut modifier is held (Shift alone just types capitals).
    fn shortcut(self) -> bool {
        self.ctrl || self.alt || self.meta
    }
}

/// What the reader thread reports. Ordinary typing carries no key information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    /// A character, space or Backspace was typed — content-free.
    Text,
    /// A navigation/editing key (arrows, Home/End, PgUp/PgDn, Insert, Delete):
    /// part of an ongoing text entry; on its own only Delete makes a step.
    Edit { delete: bool },
    /// A key that makes a step of its own: Enter, Tab, Esc, an F-key, or any
    /// key pressed together with Ctrl/Alt/Super.
    Press { mods: Mods, code: u16 },
}

impl KeyEvent {
    /// Enter / Tab without shortcut modifiers — these may close a text entry
    /// as “…, then pressed Enter” instead of becoming a separate step.
    pub fn ends_text_entry(self) -> bool {
        matches!(self, KeyEvent::Press { mods, code }
            if !mods.shortcut() && matches!(code, KEY_ENTER | KEY_KPENTER | KEY_TAB))
    }
}

/// Per-device modifier tracking + classification of key presses.
#[derive(Debug, Default)]
pub struct Classifier {
    ctrl: [bool; 2],
    shift: [bool; 2],
    alt: bool,
    meta: [bool; 2],
}

impl Classifier {
    fn mods(&self) -> Mods {
        Mods {
            ctrl: self.ctrl[0] || self.ctrl[1],
            alt: self.alt,
            shift: self.shift[0] || self.shift[1],
            meta: self.meta[0] || self.meta[1],
        }
    }

    /// Feed one evdev key event (`value`: 1 press, 0 release, 2 autorepeat).
    /// Returns the event to report, if any.
    pub fn feed(&mut self, code: u16, value: i32) -> Option<KeyEvent> {
        let down = value != 0;
        let slot = match code {
            KEY_LEFTCTRL => Some(&mut self.ctrl[0]),
            KEY_RIGHTCTRL => Some(&mut self.ctrl[1]),
            KEY_LEFTSHIFT => Some(&mut self.shift[0]),
            KEY_RIGHTSHIFT => Some(&mut self.shift[1]),
            KEY_LEFTALT => Some(&mut self.alt),
            KEY_LEFTMETA => Some(&mut self.meta[0]),
            KEY_RIGHTMETA => Some(&mut self.meta[1]),
            KEY_RIGHTALT => return None,
            _ => None,
        };
        if let Some(held) = slot {
            *held = down;
            return None;
        }
        if value == 0 {
            return None;
        }
        let mods = self.mods();
        let repeat = value == 2;
        if is_text_key(code) && !mods.shortcut() {
            // Holding a letter or Backspace keeps the entry going.
            return Some(KeyEvent::Text);
        }
        if repeat {
            return None; // a held Enter/shortcut is still one step
        }
        if mods.shortcut() {
            return Some(KeyEvent::Press { mods, code });
        }
        match code {
            KEY_DELETE => Some(KeyEvent::Edit { delete: true }),
            KEY_HOME | KEY_UP | KEY_PAGEUP | KEY_LEFT | KEY_RIGHT | KEY_END | KEY_DOWN
            | KEY_PAGEDOWN | KEY_INSERT => Some(KeyEvent::Edit { delete: false }),
            KEY_ENTER | KEY_KPENTER | KEY_TAB | KEY_ESC => Some(KeyEvent::Press { mods, code }),
            c if fkey_number(c).is_some() => Some(KeyEvent::Press { mods, code }),
            _ => None, // media keys, Caps Lock, Print, …
        }
    }
}

/// Keys that produce text (or delete it): the alphanumeric block, space,
/// Backspace, the keypad digits/operators and the ISO extra key.
fn is_text_key(code: u16) -> bool {
    matches!(code,
        2..=14      // 1 … = and Backspace
        | 16..=27   // Q … ]
        | 30..=41   // A … `
        | 43..=53   // \ … /
        | 55        // keypad *
        | KEY_SPACE
        | 71..=83   // keypad digits, -, +, .
        | 86        // ISO 102nd key (< > on European layouts)
        | 98 // keypad /
    )
}

/// F1–F24 → 1–24.
pub fn fkey_number(code: u16) -> Option<u16> {
    match code {
        59..=68 => Some(code - 58),    // F1–F10
        87 => Some(11),                // F11
        88 => Some(12),                // F12
        183..=194 => Some(code - 170), // F13–F24
        _ => None,
    }
}

/// Starts the keyboard readers. Events are only sent while `gate` is open.
/// Returns how many keyboards were found (0 → nothing started).
pub fn start(tx: Sender<KeyEvent>, gate: Arc<AtomicBool>) -> Result<usize> {
    watch_devices("kbd", is_keyboard, move |path, device| {
        device_loop(path, device, &tx, &gate)
    })
}

/// A device with letter keys, Space and Enter — a real keyboard, not just a
/// power button or a mouse with a couple of extra keys.
fn is_keyboard(device: &evdev::Device) -> bool {
    device.supported_keys().is_some_and(|keys| {
        keys.contains(evdev::KeyCode::KEY_A)
            && keys.contains(evdev::KeyCode::KEY_SPACE)
            && keys.contains(evdev::KeyCode::KEY_ENTER)
    })
}

fn device_loop(path: &str, mut device: evdev::Device, tx: &Sender<KeyEvent>, gate: &AtomicBool) {
    let mut classifier = Classifier::default();
    loop {
        let events = match device.fetch_events() {
            Ok(ev) => ev,
            Err(e) => {
                eprintln!("[stepshot] read error on {path}: {e} — keyboard dropped.");
                return;
            }
        };
        for ev in events {
            if ev.event_type() != evdev::EventType::KEY {
                continue;
            }
            // Always classify (keeps the modifier state right), only report
            // while the gate is open.
            if let Some(k) = classifier.feed(ev.code(), ev.value())
                && gate.load(Ordering::SeqCst)
                && tx.send(k).is_err()
            {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: u16 = 30;
    const KEY_S: u16 = 31;
    const KEY_F5: u16 = 63;

    #[test]
    fn typing_is_content_free() {
        let mut c = Classifier::default();
        assert_eq!(c.feed(KEY_A, 1), Some(KeyEvent::Text));
        assert_eq!(c.feed(KEY_A, 0), None);
        assert_eq!(c.feed(KEY_SPACE, 1), Some(KeyEvent::Text));
        assert_eq!(c.feed(KEY_BACKSPACE, 2), Some(KeyEvent::Text)); // held
        // Shift + letter is still just typing.
        c.feed(KEY_LEFTSHIFT, 1);
        assert_eq!(c.feed(KEY_A, 1), Some(KeyEvent::Text));
        // AltGr (e.g. @ on European layouts) too.
        c.feed(KEY_LEFTSHIFT, 0);
        c.feed(KEY_RIGHTALT, 1);
        assert_eq!(c.feed(3, 1), Some(KeyEvent::Text));
    }

    #[test]
    fn shortcuts_carry_their_code() {
        let mut c = Classifier::default();
        c.feed(KEY_RIGHTCTRL, 1);
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        assert_eq!(
            c.feed(KEY_S, 1),
            Some(KeyEvent::Press {
                mods: ctrl,
                code: KEY_S
            })
        );
        assert_eq!(c.feed(KEY_S, 2), None); // autorepeat ≠ another step
        c.feed(KEY_RIGHTCTRL, 0);
        assert_eq!(c.feed(KEY_S, 1), Some(KeyEvent::Text));
    }

    #[test]
    fn named_keys_and_editing() {
        let mut c = Classifier::default();
        let enter = c.feed(KEY_ENTER, 1).unwrap();
        assert!(enter.ends_text_entry());
        assert!(c.feed(KEY_TAB, 1).unwrap().ends_text_entry());
        assert!(!c.feed(KEY_ESC, 1).unwrap().ends_text_entry());
        assert!(matches!(c.feed(KEY_F5, 1), Some(KeyEvent::Press { .. })));
        assert_eq!(c.feed(KEY_LEFT, 1), Some(KeyEvent::Edit { delete: false }));
        assert_eq!(c.feed(KEY_DELETE, 1), Some(KeyEvent::Edit { delete: true }));
        assert_eq!(c.feed(58, 1), None); // Caps Lock
        // Ctrl+Enter is a shortcut, not the end of a text entry.
        c.feed(KEY_LEFTCTRL, 1);
        assert!(!c.feed(KEY_ENTER, 1).unwrap().ends_text_entry());
    }

    #[test]
    fn fkeys() {
        assert_eq!(fkey_number(59), Some(1));
        assert_eq!(fkey_number(68), Some(10));
        assert_eq!(fkey_number(88), Some(12));
        assert_eq!(fkey_number(194), Some(24));
        assert_eq!(fkey_number(KEY_A), None);
    }
}
