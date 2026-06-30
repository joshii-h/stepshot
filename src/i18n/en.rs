//! English strings — the fallback language.

use super::Strings;

pub static STRINGS: Strings = Strings {
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
    notify_no_input: "No input device — click capture is off. Add yourself to the “input” group and log back in.",
    report_heading: "Recording",
    report_started: "Started: {x}",
    report_total: "Total steps: {n}",
    report_step: "Step {n}",
    report_steps_word: "step(s)",
    report_self_contained: "self-contained",
};
