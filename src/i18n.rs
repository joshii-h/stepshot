//! Minimal, dependency-free internationalization.
//!
//! Design goals: trivially extensible, no runtime deps, compile-time complete.
//! Each language lives in its own file (`i18n/<code>.rs`) so adding one is a
//! self-contained PR that touches no other language.
//!
//! - **Add a string:** add a field to [`Strings`]. Every language file is a
//!   struct literal, so the compiler then forces *every* language to provide it.
//! - **Add a language:** create `i18n/xx.rs` with `pub static STRINGS: Strings
//!   = Strings { … };`, then register it here — declare `mod xx;`, add an `Xx`
//!   variant to [`Lang`], map it in [`strings_for`], and add its locale prefix to
//!   [`Lang::detect`]. Placeholders (`{n}`, `{title}`, `{action}`, `{element}`,
//!   `{x}`) are identical across languages and filled at the call site via
//!   `str::replace`.
//!
//! The active language is detected once via [`init`] and read with [`tr`].

mod de;
mod en;
mod es;
mod fr;
mod it;

use std::sync::OnceLock;

/// Supported languages.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
    Fr,
    Es,
    It,
}

impl Lang {
    /// Detect from the `LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG` environment,
    /// defaulting to English.
    pub fn detect() -> Self {
        let v = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok())
            .filter(|s| !s.is_empty())
            .unwrap_or_default();
        Self::from_locale(&v)
    }

    /// Map a locale value (e.g. `de_CH.UTF-8`, `de:en_US`) to a language.
    fn from_locale(v: &str) -> Self {
        let v = v.to_lowercase();
        match v.get(..2) {
            Some("de") => Lang::De,
            Some("fr") => Lang::Fr,
            Some("es") => Lang::Es,
            Some("it") => Lang::It,
            _ => Lang::En,
        }
    }
}

/// All user-facing strings. Templates use `{placeholders}` filled at call sites.
pub struct Strings {
    /// BCP-47 code for the HTML `lang` attribute.
    pub html_lang: &'static str,

    // Mouse buttons.
    pub click_left: &'static str,
    pub click_right: &'static str,
    pub click_middle: &'static str,
    pub click_double: &'static str, // action verb for a merged double-click

    // Step description.
    pub action_on: &'static str,        // "{action} on {element}"
    pub in_window: &'static str,        // "{action} in window “{title}”"
    pub in_active_window: &'static str, // "{action} in the active window"
    pub in_screen: &'static str,        // "{action} (full-screen view)" — panel/menu/desktop clicks
    pub element_generic: &'static str,  // fallback element word

    // Tray.
    pub tray_ready: &'static str,
    pub tray_recording: &'static str, // "● Recording — {n} step(s)"
    pub tray_paused: &'static str,    // "❚❚ Paused — {n} step(s)"
    pub tt_ready: &'static str,
    pub tt_recording: &'static str, // "Recording — {n} step(s)"
    pub tt_paused: &'static str,    // "Paused — {n} step(s)"
    pub menu_start: &'static str,
    pub menu_stop: &'static str,
    pub menu_pause: &'static str,
    pub menu_resume: &'static str,
    pub menu_open_folder: &'static str,
    pub menu_edit_last: &'static str,
    pub menu_quit: &'static str,

    // Notifications.
    pub notify_started: &'static str,
    pub notify_stopped: &'static str, // "Recording stopped — {n} step(s). Report saved."
    pub notify_paused: &'static str,  // "Recording paused."
    pub notify_resumed: &'static str, // "Recording resumed."
    pub notify_no_input: &'static str, // click capture unavailable (input group?)

    // Report.
    pub report_heading: &'static str,
    pub report_started: &'static str,        // "Started: {x}"
    pub report_total: &'static str,          // "Total steps: {n}"
    pub report_step: &'static str,           // "Step {n}"
    pub report_steps_word: &'static str,     // "step(s)"
    pub report_self_contained: &'static str, // "self-contained"

    // In-report editor (`stepshot edit`).
    pub edit_banner: &'static str,
    pub edit_hint: &'static str,
    pub edit_redact_element: &'static str,
    pub edit_delete: &'static str,
    pub edit_apply: &'static str,
    pub edit_done: &'static str,
    pub edit_applied: &'static str, // "Applied {n} step(s) — reloading…"
    pub edit_error: &'static str,
}

/// Maps a language to its string table (each defined in its own `i18n/*.rs`).
fn strings_for(lang: Lang) -> &'static Strings {
    match lang {
        Lang::En => &en::STRINGS,
        Lang::De => &de::STRINGS,
        Lang::Fr => &fr::STRINGS,
        Lang::Es => &es::STRINGS,
        Lang::It => &it::STRINGS,
    }
}

static CURRENT: OnceLock<&'static Strings> = OnceLock::new();

/// Detect the language once and store the active string table.
pub fn init() {
    let _ = CURRENT.set(strings_for(Lang::detect()));
}

/// Force a specific language by locale/code — used by `stepshot apply` to render
/// a rebuilt report in the language the session was recorded in. First-wins,
/// like [`init`].
pub fn init_lang(code: &str) {
    let _ = CURRENT.set(strings_for(Lang::from_locale(code)));
}

/// The active string table (English until [`init`] runs).
pub fn tr() -> &'static Strings {
    CURRENT.get().copied().unwrap_or(&en::STRINGS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_mapping() {
        assert!(Lang::from_locale("de_CH.UTF-8") == Lang::De);
        assert!(Lang::from_locale("DE") == Lang::De);
        assert!(Lang::from_locale("de:en_US") == Lang::De);
        assert!(Lang::from_locale("fr_FR.UTF-8") == Lang::Fr);
        assert!(Lang::from_locale("es_MX") == Lang::Es);
        assert!(Lang::from_locale("it_IT@euro") == Lang::It);
        assert!(Lang::from_locale("en_US.UTF-8") == Lang::En);
        assert!(Lang::from_locale("pt_BR") == Lang::En); // unsupported → fallback
        assert!(Lang::from_locale("") == Lang::En);
        assert!(Lang::from_locale("C") == Lang::En);
    }
}
