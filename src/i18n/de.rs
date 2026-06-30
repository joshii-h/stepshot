//! German strings.

use super::Strings;

pub static STRINGS: Strings = Strings {
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
    notify_no_input: "Kein Eingabegerät — Klick-Erfassung ist aus. Trag dich in die Gruppe „input“ ein und melde dich neu an.",
    report_heading: "Aufzeichnung",
    report_started: "Gestartet: {x}",
    report_total: "Schritte gesamt: {n}",
    report_step: "Schritt {n}",
    report_steps_word: "Schritt(e)",
    report_self_contained: "eigenständig",
};
