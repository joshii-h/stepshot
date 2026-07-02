//! Recording session: the per-session state, the per-click capture step, and
//! writing the final report. The tray event loop in `main` drives these.

use crate::a11y::Atspi;
use crate::annotate::{self, MarkerStyle};
use crate::capture::{KdeCapturer, WindowCapturer};
use crate::config::{Config, ExportConfig};
use crate::cursor::KwinCursor;
use crate::json::Json;
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

/// Writes the session report (no-op for 0 steps), honoring the configured
/// export format selection.
pub fn finalize(s: &Session, export: &ExportConfig) {
    if s.steps.is_empty() {
        return;
    }
    // The JSON source-of-truth: what `stepshot apply` rebuilds every format from.
    write_session_json(s);
    // Self-contained HTML (images embedded) — a single file you can send.
    if let Err(e) = report::write_final(&s.dir, &s.steps, &s.started, export) {
        eprintln!("[stepshot] could not write report: {e:#}");
    } else {
        eprintln!("[stepshot] report: {}", s.dir.join("report.html").display());
    }
}

/// Serialize the session to the `session.json` source-of-truth: everything
/// `stepshot apply` needs to rebuild every export, and everything the in-report
/// editor needs (per-step metadata + the clicked element's box in image pixels).
pub fn session_json(s: &Session) -> String {
    let steps: Vec<Json> = s.steps.iter().map(step_json).collect();
    Json::obj(vec![
        ("schema", 1u32.into()),
        ("app", "stepshot".into()),
        ("version", env!("CARGO_PKG_VERSION").into()),
        ("language", crate::i18n::tr().html_lang.into()),
        ("started", s.started.as_str().into()),
        ("steps", Json::Arr(steps)),
    ])
    .to_pretty()
}

fn step_json(step: &Step) -> Json {
    let button = match step.button {
        Button::Left => "left",
        Button::Right => "right",
        Button::Middle => "middle",
    };
    let element_box = match step.element_box {
        Some([x, y, w, h]) => Json::Arr(vec![x.into(), y.into(), w.into(), h.into()]),
        None => Json::Null,
    };
    Json::obj(vec![
        ("index", step.index.into()),
        ("button", button.into()),
        ("double", step.double.into()),
        ("time", step.time.as_str().into()),
        ("image", step.image_file.as_str().into()),
        ("is_screen", step.is_screen.into()),
        ("window_title", step.window_title.clone().into()),
        ("process", step.process.clone().into()),
        ("element", step.element.clone().into()),
        ("element_box", element_box),
        // The auto-generated description. The editor may override the display
        // text, but this original stays here so an override can be reverted.
        ("description", step.describe().into()),
    ])
}

/// Write `session.json` (no-op for 0 steps). Called incrementally after each
/// step (crash-safe) and again on finalize.
pub fn write_session_json(s: &Session) {
    if s.steps.is_empty() {
        return;
    }
    if let Err(e) = std::fs::write(s.dir.join("session.json"), session_json(s)) {
        eprintln!("[stepshot] could not write session.json: {e}");
    }
}

/// Map an AT-SPI element box (screen coords) into image-pixel coordinates using
/// the same frame offset + scale as the click marker, clamped to the image.
/// Returns `None` if the box is empty or maps entirely off the image.
fn map_element_box(
    (ex, ey, ew, eh): (i32, i32, i32, i32),
    (frame_x, frame_y): (i32, i32),
    scale: f64,
    (off_x, off_y): (f64, f64),
    (img_w, img_h): (u32, u32),
) -> Option<[u32; 4]> {
    if ew <= 0 || eh <= 0 {
        return None;
    }
    let to_img = |sx: i32, sy: i32| {
        (
            (sx - frame_x) as f64 * scale + off_x,
            (sy - frame_y) as f64 * scale + off_y,
        )
    };
    let (x0, y0) = to_img(ex, ey);
    let (x1, y1) = to_img(ex + ew, ey + eh);
    let cx0 = x0.clamp(0.0, img_w as f64);
    let cy0 = y0.clamp(0.0, img_h as f64);
    let cx1 = x1.clamp(0.0, img_w as f64);
    let cy1 = y1.clamp(0.0, img_h as f64);
    let (w, h) = (cx1 - cx0, cy1 - cy0);
    if w < 1.0 || h < 1.0 {
        return None;
    }
    Some([
        cx0.round() as u32,
        cy0.round() as u32,
        w.round() as u32,
        h.round() as u32,
    ])
}

/// Captures one step: get cursor → photograph window/screen → resolve element
/// → draw marker → save.
pub fn capture_step(
    index: usize,
    button: Button,
    dir: &Path,
    capturer: &KdeCapturer,
    cursor: &Option<KwinCursor>,
    atspi: &Option<Atspi>,
    marker: &MarkerStyle,
) -> Result<Step> {
    let ci = cursor.as_ref().and_then(|c| c.fetch());

    // A window capture only shows the active window's own surface. It misses
    // the click whenever the click didn't go there: panel/desktop clicks (the
    // panel never becomes the "active window") and clicks inside popups
    // (context menus are separate Wayland surfaces, invisible in a capture of
    // their parent). In those cases capture the monitor under the cursor.
    let needs_screen = ci
        .as_ref()
        .is_some_and(|c| c.in_popup || !c.in_active_window());

    let mut cap = if needs_screen {
        let mut cap =
            capture_cursor_screen(capturer, ci.as_ref()).context("screen capture failed")?;
        cap.is_screen = true;
        cap
    } else {
        capturer.capture_active_window().context("capture failed")?
    };

    // An invisible active window (KWin's Xwayland video bridge, or the bare
    // desktop) yields a blank image. Fall back to the monitor under the cursor
    // so what you actually clicked is captured.
    if !cap.is_screen && crate::capture::is_blank(&cap.image) {
        cap = capture_cursor_screen(capturer, ci.as_ref()).context("screen capture failed")?;
        cap.is_screen = true;
    }

    let el = match (atspi.as_ref(), ci.as_ref()) {
        (Some(a), Some(c)) => a.element_at(c.x, c.y),
        _ => None,
    };
    let element = el.as_ref().map(|e| e.describe());

    // For a full-screen fallback the window-relative marker math doesn't apply;
    // the baked-in cursor (include-cursor) already marks the spot.
    let mut element_box = None;
    if let Some(c) = ci.as_ref()
        && !cap.is_screen
    {
        let s = if cap.scale > 0.0 { cap.scale } else { 1.0 };
        let (img_w, img_h) = (cap.image.width(), cap.image.height());
        let off_x = (img_w as f64 - c.frame_w as f64 * s) / 2.0;
        let off_y = (img_h as f64 - c.frame_h as f64 * s) / 2.0;
        let mx = ((c.x - c.frame_x) as f64 * s + off_x).round() as i32;
        let my = ((c.y - c.frame_y) as f64 * s + off_y).round() as i32;
        annotate::draw_click_marker(&mut cap.image, mx, my, marker);
        // Map the AT-SPI element box (screen coords) into image pixels, using
        // the very same offset/scale, so the editor can redact it precisely.
        if let Some((ex, ey, ew, eh)) = el.as_ref().and_then(|e| e.bounds) {
            element_box = map_element_box(
                (ex, ey, ew, eh),
                (c.frame_x, c.frame_y),
                s,
                (off_x, off_y),
                (img_w, img_h),
            );
        }
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
        process: cap.process,
        element,
        element_box,
        is_screen: cap.is_screen,
        double: false,
    })
}

/// Capture the monitor under the cursor; if its output name is unknown, the
/// active screen as a last resort.
fn capture_cursor_screen(
    capturer: &KdeCapturer,
    ci: Option<&crate::cursor::CursorInfo>,
) -> Result<crate::capture::Capture> {
    match ci {
        Some(c) if !c.screen.is_empty() => capturer.capture_screen(&c.screen),
        _ => capturer.capture_active_screen(),
    }
}

/// Removes the trailing steps produced by the stop gesture itself: the click
/// on the tray icon (panel → full-screen capture, no window title) and the
/// click on the stop/quit menu item (popup → full-screen capture). AT-SPI
/// can't identify these clicks (plasmashell exposes them only as generic
/// "layered pane"), so this trims by their capture shape instead.
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

/// Base folder for sessions. Precedence: a CLI path argument, then the config
/// `output_dir`, then `~/Pictures/stepshot`.
pub fn output_base(config: &Config) -> Result<PathBuf> {
    if let Some(arg) = std::env::args().nth(1)
        && !arg.starts_with('-')
    {
        return Ok(PathBuf::from(arg));
    }
    if let Some(dir) = &config.output_dir {
        return Ok(expand_tilde(dir));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Pictures").join("stepshot"))
}

/// Expand a leading `~/` (or bare `~`) to `$HOME`; leave anything else as-is.
fn expand_tilde(p: &Path) -> PathBuf {
    let Some(s) = p.to_str() else {
        return p.to_path_buf();
    };
    let Some(home) = std::env::var_os("HOME") else {
        return p.to_path_buf();
    };
    if s == "~" {
        PathBuf::from(home)
    } else if let Some(rest) = s.strip_prefix("~/") {
        PathBuf::from(home).join(rest)
    } else {
        p.to_path_buf()
    }
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
            process: None,
            element: None,
            element_box: None,
            is_screen,
            double: false,
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

    #[test]
    fn maps_element_box_with_scale_and_clamp() {
        // scale 2, no centering offset, frame at origin.
        assert_eq!(
            map_element_box((10, 20, 5, 6), (0, 0), 2.0, (0.0, 0.0), (1000, 1000)),
            Some([20, 40, 10, 12])
        );
        // Frame offset is subtracted; the centering offset is added back.
        assert_eq!(
            map_element_box((100, 100, 4, 4), (100, 100), 1.0, (7.0, 8.0), (1000, 1000)),
            Some([7, 8, 4, 4])
        );
        // Degenerate and off-image boxes yield nothing.
        assert!(map_element_box((0, 0, 0, 10), (0, 0), 1.0, (0.0, 0.0), (100, 100)).is_none());
        assert!(map_element_box((-50, -50, 10, 10), (0, 0), 1.0, (0.0, 0.0), (100, 100)).is_none());
    }

    #[test]
    fn session_json_captures_key_fields() {
        let mut s0 = step(false, Some("Editor"));
        s0.process = Some("kitty".into());
        s0.element_box = Some([1, 2, 3, 4]);
        let sess = Session {
            dir: PathBuf::from("/tmp/x"),
            started: "2026-07-02 13:00:00".into(),
            steps: vec![s0],
        };
        let j = Json::parse(&session_json(&sess)).unwrap();
        assert_eq!(j.get("schema").and_then(Json::as_i64), Some(1));
        let steps = j.get("steps").and_then(Json::as_array).unwrap();
        assert_eq!(steps.len(), 1);
        let st = &steps[0];
        assert_eq!(st.get("button").and_then(Json::as_str), Some("left"));
        assert_eq!(st.get("process").and_then(Json::as_str), Some("kitty"));
        assert_eq!(
            st.get("window_title").and_then(Json::as_str),
            Some("Editor")
        );
        let bx: Vec<i64> = st
            .get("element_box")
            .and_then(Json::as_array)
            .unwrap()
            .iter()
            .filter_map(Json::as_i64)
            .collect();
        assert_eq!(bx, vec![1, 2, 3, 4]);
        assert_eq!(
            st.get("description").and_then(Json::as_str),
            Some("Left click in window “Editor”")
        );
    }
}
