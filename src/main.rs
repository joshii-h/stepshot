//! stepshot — a step recorder living in the system tray.
//!
//! Runs in the system tray. Recording is started/stopped from the tray menu.
//! On each click it photographs the active window (KWin ScreenShot2), marks the
//! click location, names the clicked element via AT-SPI; at the end it produces
//! an HTML/Markdown report. KDE Plasma / Wayland (first cut).

mod a11y;
mod annotate;
mod apply;
mod capture;
mod config;
mod cursor;
mod export_docx;
mod export_pdf;
mod i18n;
mod icon;
mod input;
mod json;
mod model;
mod notify;
mod report;
mod selftest;
mod session;
mod tray;

use a11y::Atspi;
use anyhow::{Context, Result};
use capture::KdeCapturer;
use chrono::Local;
use config::Config;
use cursor::KwinCursor;
use input::{ClickSource, EvdevClickSource};
use ksni::blocking::TrayMethods;
use selftest::run_test_modes;
use session::{Session, capture_step, finalize, output_base, write_session_json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tray::{Cmd, StepshotTray};

const USAGE: &str = "\
stepshot — step recorder for KDE/Wayland (tray app)

Usage: stepshot [OUTPUT_DIR]
       stepshot apply <SESSION_DIR> [EDITS_JSON]

Arguments:
  OUTPUT_DIR   base folder for sessions (default: ~/Pictures/stepshot)

Commands:
  apply        rebuild a session's reports, applying an editor's edits.json
               (or, with no edits.json, just regenerate every enabled export)

Options:
  -h, --help       print this help
  -V, --version    print the version
      --write-config  write a commented example config and exit";

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return Ok(());
        }
        Some("-V" | "--version") => {
            println!("stepshot {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--write-config") => {
            return write_config();
        }
        Some("apply") => {
            let mut rest = std::env::args().skip(2);
            let Some(dir) = rest.next() else {
                eprintln!("usage: stepshot apply <SESSION_DIR> [EDITS_JSON]");
                std::process::exit(2);
            };
            let edits = rest.next();
            return apply::run(
                std::path::Path::new(&dir),
                edits.as_deref().map(std::path::Path::new),
            );
        }
        Some(flag) if flag.starts_with('-') => {
            eprintln!("unknown option: {flag}\n\n{USAGE}");
            std::process::exit(2);
        }
        _ => {}
    }

    i18n::init();
    let config = Config::load();

    let capturer = KdeCapturer::connect()?;
    let source = EvdevClickSource;
    let cursor = KwinCursor::new().ok();
    let mut atspi = Atspi::connect().ok();

    if run_test_modes(&capturer, &cursor, &mut atspi, &config)? {
        return Ok(());
    }

    let base = output_base(&config)?;

    // Shared state with the tray.
    let recording = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));
    let steps_count = Arc::new(AtomicUsize::new(0));
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();

    let handle = StepshotTray {
        tx: cmd_tx.clone(),
        recording: recording.clone(),
        paused: paused.clone(),
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
            let _ = cmd_tx.send(Cmd::Terminate);
        });
    }

    eprintln!("stepshot is running in the tray — start/stop recording from the tray icon.");

    let mut session: Option<Session> = None;
    let mut last_dir: Option<PathBuf> = None;
    // The last recorded click (button + time), for double-click merging.
    let mut last_click: Option<(model::Button, Instant)> = None;
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
                    last_click = None;
                    steps_count.store(0, Ordering::SeqCst);
                    paused.store(false, Ordering::SeqCst);
                    recording.store(true, Ordering::SeqCst);
                    handle.update(|_| {});
                    // Don't record the clicks on the tray menu itself.
                    drain_clicks(&click_rx);
                    if let Some(c) = &notify_conn {
                        notify::notify(c, "stepshot", i18n::tr().notify_started, "stepshot");
                    }
                }
                Cmd::Stop => {
                    if let Some(mut s) = session.take() {
                        // Discard the gesture clicks that were still queued and
                        // trim the ones already captured as steps.
                        let pending = drain_clicks(&click_rx);
                        for dropped in session::trim_stop_gesture(&mut s.steps, pending) {
                            let _ = std::fs::remove_file(s.dir.join(&dropped.image_file));
                        }
                        finalize(&s, &config.export);
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
                Cmd::TogglePause => {
                    if session.is_some() {
                        let now_paused = !paused.load(Ordering::SeqCst);
                        paused.store(now_paused, Ordering::SeqCst);
                        // The clicks operating this menu must not become steps,
                        // and a merge must not span the pause boundary.
                        drain_clicks(&click_rx);
                        last_click = None;
                        handle.update(|_| {});
                        if let Some(c) = &notify_conn {
                            let msg = if now_paused {
                                i18n::tr().notify_paused
                            } else {
                                i18n::tr().notify_resumed
                            };
                            notify::notify(c, "stepshot", msg, "stepshot");
                        }
                    }
                }
                Cmd::OpenFolder => {
                    if let Some(d) = &last_dir {
                        let _ = std::process::Command::new("xdg-open").arg(d).spawn();
                    }
                }
                Cmd::Quit | Cmd::Terminate => {
                    if let Some(mut s) = session.take() {
                        // Only a tray-initiated quit ends with tray clicks.
                        if cmd == Cmd::Quit {
                            let pending = drain_clicks(&click_rx);
                            for dropped in session::trim_stop_gesture(&mut s.steps, pending) {
                                let _ = std::fs::remove_file(s.dir.join(&dropped.image_file));
                            }
                        }
                        finalize(&s, &config.export);
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
                    let now = Instant::now();
                    // Skip while paused or when this button isn't recorded.
                    if paused.load(Ordering::SeqCst) || !config.capture.records(click.button) {
                        // dropped — no step, last_click untouched
                    } else if is_double_click(last_click, click.button, now, &config) {
                        // Two rapid clicks of the same button → one double-click
                        // step. Upgrade the previous step instead of capturing a
                        // near-identical second screenshot.
                        if let Some(step) = s.steps.last_mut() {
                            step.double = true;
                        }
                        let _ = report::write_reports(&s.dir, &s.steps, &s.started, &config.export);
                        write_session_json(s);
                        last_click = None; // don't chain a third click into it
                    } else {
                        let index = s.steps.len() + 1;
                        match capture_step(
                            index,
                            click.button,
                            &s.dir,
                            &capturer,
                            &cursor,
                            &atspi,
                            &config.marker,
                        ) {
                            Ok(step) => {
                                s.steps.push(step);
                                steps_count.store(s.steps.len(), Ordering::SeqCst);
                                let _ = report::write_reports(
                                    &s.dir,
                                    &s.steps,
                                    &s.started,
                                    &config.export,
                                );
                                write_session_json(s);
                                last_click = Some((click.button, now));
                            }
                            Err(e) => eprintln!("[stepshot] step {index}: {e:#}"),
                        }
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

/// Whether `button` clicked at `now` completes a double-click with the last
/// recorded click — same button, within the configured window (0 disables it).
fn is_double_click(
    last: Option<(model::Button, Instant)>,
    button: model::Button,
    now: Instant,
    config: &Config,
) -> bool {
    let window = config.capture.double_click_ms;
    if window == 0 {
        return false;
    }
    match last {
        Some((b, t)) => b == button && now.duration_since(t).as_millis() as u64 <= window,
        None => false,
    }
}

/// Drain all queued clicks, returning how many were discarded.
fn drain_clicks(rx: &mpsc::Receiver<model::Click>) -> usize {
    let mut n = 0;
    while rx.try_recv().is_ok() {
        n += 1;
    }
    n
}

/// `--write-config`: write the commented example config to the standard path
/// (never overwriting an existing one) and print where it went.
fn write_config() -> Result<()> {
    let path = config::config_path().context("could not determine the config path")?;
    if path.exists() {
        eprintln!("config already exists: {}", path.display());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("could not create the config folder")?;
    }
    std::fs::write(&path, Config::example_toml()).context("could not write the config file")?;
    println!("wrote example config: {}", path.display());
    Ok(())
}
