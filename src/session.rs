//! Recording session: the per-session state, the per-click capture step, and
//! writing the final report. Platform-neutral — it talks only to the traits in
//! [`crate::platform`]; the per-OS event loops (`run_linux`, `win::run`) drive it.

use crate::annotate;
use crate::model::{Button, Click, Step};
use crate::platform::{CursorTracker, ElementResolver, WindowCapturer, is_blank};
use crate::report;
use anyhow::{Context, Result};
use chrono::Local;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

/// A running recording session.
pub struct Session {
    pub dir: PathBuf,
    pub started: String,
    pub steps: Vec<Step>,
}

/// Writes the session report (no-op for 0 steps).
pub fn finalize(s: &Session) {
    if s.steps.is_empty() {
        return;
    }
    // Self-contained HTML (images embedded) — a single file you can send.
    if let Err(e) = report::write_final(&s.dir, &s.steps, &s.started) {
        eprintln!("[stepshot] could not write report: {e:#}");
    } else {
        eprintln!("[stepshot] report: {}", s.dir.join("report.html").display());
    }
}

/// Captures one step: get cursor → photograph window/screen → resolve element
/// → draw marker → save.
pub fn capture_step(
    index: usize,
    button: Button,
    dir: &Path,
    capturer: &dyn WindowCapturer,
    cursor: Option<&dyn CursorTracker>,
    resolver: Option<&dyn ElementResolver>,
) -> Result<Step> {
    let ci = cursor.and_then(|c| c.fetch());

    // A window capture only shows the active window's own surface. It misses
    // the click whenever the click didn't go there: panel/desktop clicks (the
    // panel never becomes the "active window") and clicks inside popups
    // (context menus are separate surfaces, invisible in a capture of their
    // parent). In those cases capture the monitor under the cursor; if the
    // backend has no screen path, fall back to the window capture.
    let needs_screen = ci
        .as_ref()
        .is_some_and(|c| c.in_popup || !c.in_active_window());

    let mut cap = if needs_screen {
        match capturer.capture_screen_under_cursor(ci.as_ref()) {
            Ok(mut c) => {
                c.is_screen = true;
                c
            }
            Err(e) => {
                eprintln!("[stepshot] screen capture unavailable ({e:#}); using the window");
                capturer.capture_active_window().context("capture failed")?
            }
        }
    } else {
        capturer.capture_active_window().context("capture failed")?
    };

    // An invisible active window (KWin's Xwayland video bridge, or the bare
    // desktop) yields a blank image. Fall back to the monitor under the cursor
    // so what you actually clicked is captured.
    if !cap.is_screen && is_blank(&cap.image) {
        cap = capturer
            .capture_screen_under_cursor(ci.as_ref())
            .context("screen capture failed")?;
        cap.is_screen = true;
    }

    let element = match (resolver, ci.as_ref()) {
        (Some(a), Some(c)) => a.element_at(c.x, c.y).map(|e| e.describe()),
        _ => None,
    };

    // For a full-screen capture the window-relative marker math doesn't apply;
    // the backend makes the click visible there (baked-in cursor or marker).
    if let Some(c) = ci.as_ref()
        && !cap.is_screen
    {
        let s = if cap.scale > 0.0 { cap.scale } else { 1.0 };
        let off_x = (cap.image.width() as f64 - c.frame_w as f64 * s) / 2.0;
        let off_y = (cap.image.height() as f64 - c.frame_h as f64 * s) / 2.0;
        let mx = ((c.x - c.frame_x) as f64 * s + off_x).round() as i32;
        let my = ((c.y - c.frame_y) as f64 * s + off_y).round() as i32;
        annotate::draw_click_marker(&mut cap.image, mx, my);
    }

    let image_file = format!("step-{index:03}.png");
    cap.image
        .save(dir.join(&image_file))
        .with_context(|| format!("could not save image {image_file}"))?;

    Ok(Step {
        index,
        button,
        time: Local::now().format("%H:%M:%S").to_string(),
        image_file,
        window_title: cap.window_title,
        element,
        is_screen: cap.is_screen,
    })
}

/// Removes the trailing steps produced by the stop gesture itself: the click
/// on the tray icon (panel → full-screen capture, no window title) and the
/// click on the stop/quit menu item (popup → full-screen capture). The
/// accessibility layer can't identify these clicks reliably (plasmashell
/// exposes them only as generic "layered pane"), so this trims by their
/// capture shape instead.
///
/// `pending` is the number of clicks that were still unprocessed when the
/// command arrived and were discarded before capture — each one is a gesture
/// click that never became a step, so it reduces how many steps to trim.
///
/// Returns the removed steps so the caller can delete their screenshots.
pub fn trim_stop_gesture(steps: &mut Vec<Step>, pending: usize) -> Vec<Step> {
    let mut removed = Vec::new();
    for _ in 0..2usize.saturating_sub(pending) {
        if steps
            .last()
            .is_some_and(|s| s.is_screen && s.window_title.is_none())
        {
            removed.extend(steps.pop());
        } else {
            break;
        }
    }
    removed
}

/// Drain all queued clicks, returning how many were discarded.
pub fn drain_clicks(rx: &Receiver<Click>) -> usize {
    let mut n = 0;
    while rx.try_recv().is_ok() {
        n += 1;
    }
    n
}

/// Base folder for sessions: optional CLI argument, otherwise ~/Pictures/stepshot.
pub fn output_base() -> Result<PathBuf> {
    if let Some(arg) = std::env::args().nth(1)
        && !arg.starts_with('-')
    {
        return Ok(PathBuf::from(arg));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .context("neither HOME nor USERPROFILE is set")?;
    Ok(PathBuf::from(home).join("Pictures").join("stepshot"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(is_screen: bool, window_title: Option<&str>) -> Step {
        Step {
            index: 1,
            button: Button::Left,
            time: "12:00:00".into(),
            image_file: "step-001.png".into(),
            window_title: window_title.map(String::from),
            element: None,
            is_screen,
        }
    }

    #[test]
    fn trims_the_two_gesture_steps() {
        // window click, tray-icon click, menu-item click
        let mut steps = vec![
            step(false, Some("Editor")),
            step(true, None),
            step(true, None),
        ];
        trim_stop_gesture(&mut steps, 0);
        assert_eq!(steps.len(), 1);
    }

    #[test]
    fn keeps_a_legit_screen_step_before_the_gesture() {
        // start-menu click (legit), then the two gesture clicks
        let mut steps = vec![step(true, None), step(true, None), step(true, None)];
        trim_stop_gesture(&mut steps, 0);
        assert_eq!(steps.len(), 1); // trims at most two
    }

    #[test]
    fn stops_at_window_steps() {
        let mut steps = vec![step(false, Some("Editor")), step(false, Some("Editor"))];
        trim_stop_gesture(&mut steps, 0);
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn pending_clicks_reduce_the_trim() {
        // The menu-item click was still in the channel (never captured):
        // only the tray-icon step exists and only it may be trimmed.
        let mut steps = vec![step(true, None), step(true, None)];
        trim_stop_gesture(&mut steps, 1);
        assert_eq!(steps.len(), 1);
        // Both gesture clicks pending → nothing to trim.
        let mut steps = vec![step(true, None)];
        trim_stop_gesture(&mut steps, 2);
        assert_eq!(steps.len(), 1);
    }

    #[test]
    fn screen_steps_with_title_are_not_gesture_clicks() {
        let mut steps = vec![step(true, Some("Some Window"))];
        trim_stop_gesture(&mut steps, 0);
        assert_eq!(steps.len(), 1);
    }
}
