//! Recording session: the per-session state, the per-click capture step, and
//! writing the final report. The tray event loop in `main` drives these.

use crate::a11y::Atspi;
use crate::annotate;
use crate::capture::{KdeCapturer, WindowCapturer};
use crate::cursor::KwinCursor;
use crate::model::{Button, Step};
use crate::report;
use anyhow::{Context, Result};
use chrono::Local;
use std::path::{Path, PathBuf};

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

/// Captures one step: get cursor → photograph window → resolve element
/// → draw marker → save.
pub fn capture_step(
    index: usize,
    button: Button,
    dir: &Path,
    capturer: &KdeCapturer,
    cursor: &Option<KwinCursor>,
    atspi: &Option<Atspi>,
) -> Result<Step> {
    let ci = cursor.as_ref().and_then(|c| c.fetch());

    let mut cap = capturer.capture_active_window().context("capture failed")?;

    let element = match (atspi.as_ref(), ci) {
        (Some(a), Some(c)) => a.element_at(c.x, c.y).map(|e| e.describe()),
        _ => None,
    };

    // For a full-screen fallback the window-relative marker math doesn't apply;
    // the baked-in cursor (include-cursor) already marks the spot.
    if let Some(c) = ci
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
    })
}

/// Base folder for sessions: optional CLI argument, otherwise ~/Pictures/stepshot.
pub fn output_base() -> Result<PathBuf> {
    if let Some(arg) = std::env::args().nth(1)
        && !arg.starts_with('-')
    {
        return Ok(PathBuf::from(arg));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Pictures").join("stepshot"))
}
