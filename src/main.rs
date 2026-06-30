//! stepshot — a step recorder living in the system tray.
//!
//! Runs in the system tray. Recording is started/stopped from the tray menu.
//! On each click it photographs the active window (KWin ScreenShot2), marks the
//! click location, names the clicked element via AT-SPI; at the end it produces
//! an HTML/Markdown report. KDE Plasma / Wayland (first cut).

mod a11y;
mod annotate;
mod capture;
mod cursor;
mod i18n;
mod icon;
mod input;
mod model;
mod notify;
mod report;
mod tray;

use a11y::Atspi;
use anyhow::{Context, Result};
use capture::{KdeCapturer, WindowCapturer};
use chrono::Local;
use cursor::KwinCursor;
use input::{ClickSource, EvdevClickSource};
use ksni::blocking::TrayMethods;
use model::{Button, Step};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use tray::{Cmd, StepshotTray};

/// A running recording session.
struct Session {
    dir: PathBuf,
    started: String,
    steps: Vec<Step>,
}

fn main() -> Result<()> {
    i18n::init();

    let capturer = KdeCapturer::connect()?;
    let source = EvdevClickSource;
    let cursor = KwinCursor::new().ok();
    let mut atspi = Atspi::connect().ok();

    if run_test_modes(&capturer, &cursor, &mut atspi)? {
        return Ok(());
    }

    let base = output_base()?;

    // Shared state with the tray.
    let recording = Arc::new(AtomicBool::new(false));
    let steps_count = Arc::new(AtomicUsize::new(0));
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();

    let handle = StepshotTray {
        tx: cmd_tx.clone(),
        recording: recording.clone(),
        steps: steps_count.clone(),
    }
    .spawn()
    .context("could not create tray icon (is a StatusNotifierWatcher / KDE panel running?)")?;

    // Connection for notifications.
    let notify_conn = zbus::blocking::Connection::session().ok();

    // Click source. If it can't start (typically: user not in the `input` group),
    // we keep the tray alive and notify instead of exiting — otherwise the app
    // would vanish with no window and no icon, looking like a broken tray.
    // `keepalive_tx` holds the channel open so the main loop never sees a
    // disconnect even when no device thread owns a sender.
    let (keepalive_tx, click_rx) = mpsc::channel();
    if let Err(e) = source.start(keepalive_tx.clone()) {
        eprintln!("[stepshot] click capture unavailable: {e:#}");
        if let Some(c) = &notify_conn {
            notify::notify(c, "stepshot", i18n::tr().notify_no_input, "stepshot");
        }
    }

    // Ctrl+C also quits the app (fallback).
    {
        let cmd_tx = cmd_tx.clone();
        let _ = ctrlc::set_handler(move || {
            let _ = cmd_tx.send(Cmd::Quit);
        });
    }

    eprintln!("stepshot is running in the tray — start/stop recording from the tray icon.");

    let mut session: Option<Session> = None;
    let mut last_dir: Option<PathBuf> = None;
    let mut run = true;

    while run {
        // Handle control commands first.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Cmd::Start if session.is_none() => {
                    let dir = base.join(format!(
                        "session-{}",
                        Local::now().format("%Y-%m-%d_%H-%M-%S")
                    ));
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        eprintln!("[stepshot] session folder: {e}");
                        continue;
                    }
                    if let Some(a) = atspi.as_mut() {
                        a.enable();
                    }
                    session = Some(Session {
                        dir: dir.clone(),
                        started: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                        steps: Vec::new(),
                    });
                    last_dir = Some(dir);
                    steps_count.store(0, Ordering::SeqCst);
                    recording.store(true, Ordering::SeqCst);
                    handle.update(|_| {});
                    // Don't record the click on the tray menu itself.
                    while click_rx.try_recv().is_ok() {}
                    if let Some(c) = &notify_conn {
                        notify::notify(c, "stepshot", i18n::tr().notify_started, "stepshot");
                    }
                }
                Cmd::Stop => {
                    if let Some(s) = session.take() {
                        finalize(&s);
                        if let Some(a) = atspi.as_ref() {
                            a.restore();
                        }
                        recording.store(false, Ordering::SeqCst);
                        handle.update(|_| {});
                        if let Some(c) = &notify_conn {
                            let msg = i18n::tr()
                                .notify_stopped
                                .replace("{n}", &s.steps.len().to_string());
                            notify::notify(c, "stepshot", &msg, "stepshot");
                        }
                    }
                }
                Cmd::OpenFolder => {
                    if let Some(d) = &last_dir {
                        let _ = std::process::Command::new("xdg-open").arg(d).spawn();
                    }
                }
                Cmd::Quit => {
                    if let Some(s) = session.take() {
                        finalize(&s);
                        if let Some(a) = atspi.as_ref() {
                            a.restore();
                        }
                    }
                    run = false;
                }
                Cmd::Start => {} // already recording
            }
        }
        if !run {
            break;
        }

        // Process clicks (with a timeout so commands are handled promptly).
        match click_rx.recv_timeout(Duration::from_millis(150)) {
            Ok(click) => {
                if let Some(s) = session.as_mut() {
                    let index = s.steps.len() + 1;
                    match capture_step(index, click.button, &s.dir, &capturer, &cursor, &atspi) {
                        Ok(step) => {
                            s.steps.push(step);
                            steps_count.store(s.steps.len(), Ordering::SeqCst);
                            let _ = report::write_reports(&s.dir, &s.steps, &s.started);
                        }
                        Err(e) => eprintln!("[stepshot] step {index}: {e:#}"),
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => run = false,
        }
    }

    let _ = handle.shutdown();
    eprintln!("stepshot stopped.");
    Ok(())
}

/// Writes the session report (no-op for 0 steps).
fn finalize(s: &Session) {
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
fn capture_step(
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

/// Debug/self-test modes (env-driven). Returns true if handled.
fn run_test_modes(
    capturer: &KdeCapturer,
    cursor: &Option<KwinCursor>,
    atspi: &mut Option<Atspi>,
) -> Result<bool> {
    if std::env::var_os("STEPSHOT_ICON").is_some() {
        icon::debug_png(false, 128)
            .save("/tmp/stepshot-icon-idle.png")
            .ok();
        icon::debug_png(true, 128)
            .save("/tmp/stepshot-icon-rec.png")
            .ok();
        println!("Icons → /tmp/stepshot-icon-idle.png, /tmp/stepshot-icon-rec.png");
        return Ok(true);
    }
    if std::env::var_os("STEPSHOT_ATTREE").is_some() {
        if let Some(a) = atspi.as_mut() {
            a.enable();
            std::thread::sleep(Duration::from_millis(1500));
            let depth = std::env::var("STEPSHOT_ATTREE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3);
            a.debug_dump(depth);
            a.restore();
        }
        return Ok(true);
    }
    if std::env::var_os("STEPSHOT_ATDUMP").is_some() {
        if let Some(a) = atspi.as_mut() {
            a.enable();
            std::thread::sleep(Duration::from_millis(1500));
            match a.debug_first_button() {
                Some((name, cx, cy)) => {
                    println!("button “{name}” @ ({cx},{cy})");
                    println!(
                        "element_at → {:?}",
                        a.element_at(cx, cy).map(|e| e.describe())
                    );
                }
                None => println!("no named button found."),
            }
            a.restore();
        }
        return Ok(true);
    }
    if std::env::var_os("STEPSHOT_ONESHOT").is_some() {
        let dir = output_base()?.join(format!("oneshot-{}", Local::now().format("%H-%M-%S")));
        std::fs::create_dir_all(&dir)?;
        if let Some(a) = atspi.as_mut() {
            a.enable();
        }
        let started = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let step = capture_step(1, Button::Left, &dir, capturer, cursor, atspi)?;
        println!("Oneshot → {}", step.describe());
        report::write_final(&dir, &[step], &started)?;
        if let Some(a) = atspi.as_ref() {
            a.restore();
        }
        println!("Report: {}", dir.join("report.html").display());
        return Ok(true);
    }
    Ok(false)
}

/// Base folder for sessions: optional CLI argument, otherwise ~/Pictures/stepshot.
fn output_base() -> Result<PathBuf> {
    if let Some(arg) = std::env::args().nth(1)
        && !arg.starts_with('-')
    {
        return Ok(PathBuf::from(arg));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Pictures").join("stepshot"))
}
