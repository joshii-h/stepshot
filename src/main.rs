//! stepshot — a step recorder living in the system tray.
//!
//! Runs in the system tray. Recording is started/stopped from the tray menu.
//! On each click it photographs the active window (or the screen when the
//! click lands on a panel/popup), marks the click location, names the clicked
//! element via the platform's accessibility API; at the end it produces an
//! HTML/Markdown report plus PDF/DOCX exports.
//!
//! The OS-specific work sits behind the traits in [`platform`]; the shared
//! session logic lives in [`session`]. Entry points: [`run_linux`] for
//! KDE/Wayland, `win::run` for Windows.

mod annotate;
mod export_docx;
mod export_pdf;
mod i18n;
mod icon;
mod model;
mod platform;
mod report;
mod session;

#[cfg(target_os = "linux")]
mod a11y;
#[cfg(target_os = "linux")]
mod capture;
#[cfg(target_os = "linux")]
mod cursor;
#[cfg(target_os = "linux")]
mod input;
#[cfg(target_os = "linux")]
mod notify;
#[cfg(target_os = "linux")]
mod selftest;
#[cfg(target_os = "linux")]
mod tray;

#[cfg(windows)]
mod win;

use anyhow::Result;

const USAGE: &str = "\
stepshot — step recorder (tray app)

Usage: stepshot [OUTPUT_DIR]

Arguments:
  OUTPUT_DIR   base folder for sessions (default: ~/Pictures/stepshot)

Options:
  -h, --help      print this help
  -V, --version   print the version";

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
        Some(flag) if flag.starts_with('-') => {
            eprintln!("unknown option: {flag}\n\n{USAGE}");
            std::process::exit(2);
        }
        _ => {}
    }

    i18n::init();
    platform_main()
}

#[cfg(target_os = "linux")]
fn platform_main() -> Result<()> {
    run_linux()
}

#[cfg(windows)]
fn platform_main() -> Result<()> {
    win::run()
}

#[cfg(not(any(target_os = "linux", windows)))]
fn platform_main() -> Result<()> {
    anyhow::bail!("stepshot has no backend for this platform yet")
}

// ─────────────────────────── Linux (KDE/Wayland) ───────────────────────────

#[cfg(target_os = "linux")]
fn run_linux() -> Result<()> {
    use a11y::Atspi;
    use anyhow::Context;
    use capture::KdeCapturer;
    use chrono::Local;
    use cursor::KwinCursor;
    use input::EvdevClickSource;
    use ksni::blocking::TrayMethods;
    use platform::{ClickSource, CursorTracker, ElementResolver};
    use selftest::run_test_modes;
    use session::{Session, capture_step, drain_clicks, finalize, output_base};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;
    use tray::{Cmd, StepshotTray};

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
            let _ = cmd_tx.send(Cmd::Terminate);
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
                Cmd::Quit | Cmd::Terminate => {
                    if let Some(mut s) = session.take() {
                        // Only a tray-initiated quit ends with tray clicks.
                        if cmd == Cmd::Quit {
                            let pending = drain_clicks(&click_rx);
                            for dropped in session::trim_stop_gesture(&mut s.steps, pending) {
                                let _ = std::fs::remove_file(s.dir.join(&dropped.image_file));
                            }
                        }
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
                    match capture_step(
                        index,
                        click.button,
                        &s.dir,
                        &capturer,
                        cursor.as_ref().map(|c| c as &dyn CursorTracker),
                        atspi.as_ref().map(|a| a as &dyn ElementResolver),
                    ) {
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
