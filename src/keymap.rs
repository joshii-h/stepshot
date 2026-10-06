//! Shortcut labels in the user's real keyboard layout (“Ctrl+Z” must not come
//! out as “Ctrl+Y” on a German/Swiss keyboard, where evdev's US-named key
//! codes for Y and Z are swapped).
//!
//! The layout comes from KDE's `kxkbrc` (layouts, variants, model, options)
//! plus the currently active layout index over D-Bus (`org.kde.keyboard`), with
//! the `XKB_DEFAULT_*` environment as fallback; libxkbcommon then maps an evdev
//! key code to the character on that key's base level.

use crate::keys::{self, KeyEvent, Mods};
use xkbcommon::xkb;

/// XKB rule names (`RMLVO`) plus the active layout index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutNames {
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: Option<String>,
    /// Index of the active layout within the comma-separated `layout` list.
    pub group: u32,
}

impl LayoutNames {
    /// Detect the current layout: KDE first, then the XKB environment.
    pub fn detect() -> Self {
        let mut names = kde_layout().unwrap_or_else(env_layout);
        names.group = kde_active_group().unwrap_or(0);
        names
    }

    /// Parse the `[Layout]` group of KDE's `kxkbrc`.
    fn from_kxkbrc(text: &str) -> Option<Self> {
        let mut in_layout = false;
        let mut names = LayoutNames::default();
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_layout = line == "[Layout]";
                continue;
            }
            let Some((k, v)) = line.split_once('=').filter(|_| in_layout) else {
                continue;
            };
            let v = v.trim().to_string();
            match k.trim() {
                "LayoutList" => names.layout = v,
                "VariantList" => names.variant = v,
                "Model" => names.model = v,
                "Options" if !v.is_empty() => names.options = Some(v),
                _ => {}
            }
        }
        (!names.layout.is_empty()).then_some(names)
    }
}

fn kde_layout() -> Option<LayoutNames> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    LayoutNames::from_kxkbrc(&std::fs::read_to_string(base.join("kxkbrc")).ok()?)
}

fn kde_active_group() -> Option<u32> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let reply = conn
        .call_method(
            Some("org.kde.keyboard"),
            "/Layouts",
            Some("org.kde.KeyboardLayouts"),
            "getLayout",
            &(),
        )
        .ok()?;
    reply.body().deserialize::<u32>().ok()
}

fn env_layout() -> LayoutNames {
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    LayoutNames {
        model: var("XKB_DEFAULT_MODEL"),
        layout: var("XKB_DEFAULT_LAYOUT"),
        variant: var("XKB_DEFAULT_VARIANT"),
        options: std::env::var("XKB_DEFAULT_OPTIONS")
            .ok()
            .filter(|s| !s.is_empty()),
        group: 0,
    }
}

/// Labels key presses for the step description.
pub struct Keymap {
    /// `None` when libxkbcommon couldn't compile the layout — letters then fall
    /// back to their (US) evdev names.
    state: Option<xkb::State>,
}

impl Keymap {
    pub fn new(names: &LayoutNames) -> Self {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let state = xkb::Keymap::new_from_names(
            &ctx,
            "",
            &names.model,
            &names.layout,
            &names.variant,
            names.options.clone(),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .map(|km| {
            let mut st = xkb::State::new(&km);
            // Select the active layout; no modifiers → each key's base level.
            st.update_mask(0, 0, 0, 0, 0, names.group);
            st
        });
        Self { state }
    }

    /// The layout's base-level character for an evdev key code, uppercased
    /// (“S”, “+”, “Ü”) — `None` for keys without a printable symbol.
    fn char_label(&self, code: u16) -> Option<String> {
        let st = self.state.as_ref()?;
        let s = st.key_get_utf8(xkb::Keycode::new(u32::from(code) + 8));
        let s = s.trim();
        (!s.is_empty() && !s.chars().any(char::is_control)).then(|| s.to_uppercase())
    }

    /// “Ctrl+Shift+T”, “Enter”, “F5” — localized modifier names.
    pub fn label(&self, mods: Mods, code: u16) -> String {
        let t = crate::i18n::tr();
        let mut parts: Vec<String> = Vec::new();
        if mods.ctrl {
            parts.push(t.key_ctrl.into());
        }
        if mods.alt {
            parts.push("Alt".into());
        }
        if mods.shift {
            parts.push(t.key_shift.into());
        }
        if mods.meta {
            parts.push("Super".into());
        }
        let key = named_key(code)
            .map(str::to_string)
            .or_else(|| keys::fkey_number(code).map(|n| format!("F{n}")))
            .or_else(|| self.char_label(code))
            .unwrap_or_else(|| format!("#{code}"));
        parts.push(key);
        parts.join("+")
    }

    /// The label for a [`KeyEvent::Press`]; `None` for other events.
    pub fn label_event(&self, ev: KeyEvent) -> Option<String> {
        match ev {
            KeyEvent::Press { mods, code } => Some(self.label(mods, code)),
            KeyEvent::Edit { delete: true } => Some(self.label(Mods::default(), keys::KEY_DELETE)),
            _ => None,
        }
    }
}

/// Keys whose label doesn't depend on the layout.
fn named_key(code: u16) -> Option<&'static str> {
    Some(match code {
        keys::KEY_ESC => "Esc",
        keys::KEY_BACKSPACE => "Backspace",
        keys::KEY_TAB => "Tab",
        keys::KEY_ENTER | keys::KEY_KPENTER => "Enter",
        keys::KEY_SPACE => "Space",
        keys::KEY_HOME => "Home",
        keys::KEY_END => "End",
        keys::KEY_PAGEUP => "PgUp",
        keys::KEY_PAGEDOWN => "PgDn",
        keys::KEY_UP => "↑",
        keys::KEY_DOWN => "↓",
        keys::KEY_LEFT => "←",
        keys::KEY_RIGHT => "→",
        keys::KEY_INSERT => "Insert",
        keys::KEY_DELETE => "Delete",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_Y: u16 = 21;
    const KEY_S: u16 = 31;

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::default()
        }
    }

    #[test]
    fn parses_kxkbrc() {
        let n = LayoutNames::from_kxkbrc(
            "[Layout]\nLayoutList=us,de\nVariantList=,nodeadkeys\nModel=pc105\nUse=true\n",
        )
        .unwrap();
        assert_eq!(n.layout, "us,de");
        assert_eq!(n.variant, ",nodeadkeys");
        assert_eq!(n.model, "pc105");
        assert_eq!(n.options, None);
        assert!(LayoutNames::from_kxkbrc("[Other]\nLayoutList=de\n").is_none());
    }

    #[test]
    fn labels_follow_the_layout() {
        let us = Keymap::new(&LayoutNames {
            layout: "us".into(),
            ..LayoutNames::default()
        });
        let de = Keymap::new(&LayoutNames {
            layout: "de".into(),
            ..LayoutNames::default()
        });
        // Skip quietly where the XKB data files aren't installed.
        if us.state.is_none() || de.state.is_none() {
            return;
        }
        assert_eq!(us.label(ctrl(), KEY_S), "Ctrl+S");
        // The key evdev calls KEY_Y is Z on a German keyboard.
        assert_eq!(us.label(ctrl(), KEY_Y), "Ctrl+Y");
        assert_eq!(de.label(ctrl(), KEY_Y), "Ctrl+Z");
        // Second layout in a list, selected via the group index.
        let both = Keymap::new(&LayoutNames {
            layout: "us,de".into(),
            group: 1,
            ..LayoutNames::default()
        });
        assert_eq!(both.label(ctrl(), KEY_Y), "Ctrl+Z");
    }

    #[test]
    fn swiss_layout_from_this_machine_style_config() {
        let ch = Keymap::new(&LayoutNames {
            layout: "ch".into(),
            ..LayoutNames::default()
        });
        if ch.state.is_none() {
            return;
        }
        assert_eq!(ch.label(ctrl(), KEY_Y), "Ctrl+Z");
        assert_eq!(ch.label(ctrl(), 26), "Ctrl+Ü"); // the key right of P
    }

    #[test]
    fn named_keys_ignore_the_layout() {
        let km = Keymap { state: None };
        assert_eq!(km.label(Mods::default(), keys::KEY_ENTER), "Enter");
        assert_eq!(km.label(Mods::default(), 63), "F5");
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        assert_eq!(km.label(shift, keys::KEY_TAB), "Shift+Tab");
    }
}
