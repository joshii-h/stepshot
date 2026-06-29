//! Minimal, dependency-free internationalization.
//!
//! Design goals: trivially extensible, no runtime deps, compile-time complete.
//!
//! - **Add a string:** add a field to [`Strings`]. Every language `static` is a
//!   struct literal, so the compiler then forces *every* language to provide it.
//! - **Add a language:** add a `static XX: Strings = Strings { … }` and one arm
//!   in [`strings_for`]. Placeholders (`{n}`, `{title}`, `{action}`, `{element}`,
//!   `{x}`) are identical across languages and filled at the call site via
//!   `str::replace`.
//!
//! The active language is detected once via [`init`] and read with [`tr`].

use std::sync::OnceLock;

/// Supported languages.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
}

impl Lang {
    /// Detect from the `LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG` environment,
    /// defaulting to English.
    pub fn detect() -> Self {
        let v = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok())
            .filter(|s| !s.is_empty())
            .unwrap_or_default()
            .to_lowercase();
        if v.starts_with("de") {
            Lang::De
        } else {
            Lang::En
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

    // Step description.
    pub action_on: &'static str,        // "{action} on {element}"
    pub in_window: &'static str,        // "{action} in window “{title}”"
    pub in_active_window: &'static str, // "{action} in the active window"
    pub element_generic: &'static str,  // fallback element word

    // Tray.
    pub tray_ready: &'static str,
    pub tray_recording: &'static str, // "● Recording — {n} step(s)"
    pub tt_ready: &'static str,
    pub tt_recording: &'static str, // "Recording — {n} step(s)"
    pub menu_start: &'static str,
    pub menu_stop: &'static str,
    pub menu_open_folder: &'static str,
    pub menu_quit: &'static str,

    // Notifications.
    pub notify_started: &'static str,
    pub notify_stopped: &'static str, // "Recording stopped — {n} step(s). Report saved."

    // Report.
    pub report_heading: &'static str,
    pub report_started: &'static str,        // "Started: {x}"
    pub report_total: &'static str,          // "Total steps: {n}"
    pub report_step: &'static str,           // "Step {n}"
    pub report_steps_word: &'static str,     // "step(s)"
    pub report_self_contained: &'static str, // "self-contained"
}

static EN: Strings = Strings {
    html_lang: "en",
    click_left: "Left click",
    click_right: "Right click",
    click_middle: "Middle click",
    action_on: "{action} on {element}",
    in_window: "{action} in window “{title}”",
    in_active_window: "{action} in the active window",
    element_generic: "element",
    tray_ready: "stepshot — ready",
    tray_recording: "● Recording — {n} step(s)",
    tt_ready: "Ready",
    tt_recording: "Recording — {n} step(s)",
    menu_start: "Start recording",
    menu_stop: "Stop recording & write report",
    menu_open_folder: "Open last report folder",
    menu_quit: "Quit stepshot",
    notify_started: "Recording started",
    notify_stopped: "Recording stopped — {n} step(s). Report saved.",
    report_heading: "Recording",
    report_started: "Started: {x}",
    report_total: "Total steps: {n}",
    report_step: "Step {n}",
    report_steps_word: "step(s)",
    report_self_contained: "self-contained",
};

static DE: Strings = Strings {
    html_lang: "de",
    click_left: "Linksklick",
    click_right: "Rechtsklick",
    click_middle: "Mittelklick",
    action_on: "{action} auf {element}",
    in_window: "{action} im Fenster „{title}“",
    in_active_window: "{action} im aktiven Fenster",
    element_generic: "Element",
    tray_ready: "stepshot — bereit",
    tray_recording: "● Aufnahme — {n} Schritt(e)",
    tt_ready: "Bereit",
    tt_recording: "Aufnahme — {n} Schritt(e)",
    menu_start: "Aufnahme starten",
    menu_stop: "Aufnahme beenden & Bericht schreiben",
    menu_open_folder: "Letzten Bericht-Ordner öffnen",
    menu_quit: "stepshot beenden",
    notify_started: "Aufnahme gestartet",
    notify_stopped: "Aufnahme beendet — {n} Schritt(e). Bericht gespeichert.",
    report_heading: "Aufzeichnung",
    report_started: "Gestartet: {x}",
    report_total: "Schritte gesamt: {n}",
    report_step: "Schritt {n}",
    report_steps_word: "Schritt(e)",
    report_self_contained: "eigenständig",
};

fn strings_for(lang: Lang) -> &'static Strings {
    match lang {
        Lang::En => &EN,
        Lang::De => &DE,
    }
}

static CURRENT: OnceLock<&'static Strings> = OnceLock::new();

/// Detect the language once and store the active string table.
pub fn init() {
    let _ = CURRENT.set(strings_for(Lang::detect()));
}

/// The active string table (English until [`init`] runs).
pub fn tr() -> &'static Strings {
    CURRENT.get().copied().unwrap_or(&EN)
}
