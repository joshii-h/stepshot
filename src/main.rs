//! stepshot — a step recorder living in the system tray.
//!
//! Runs in the system tray. Recording is started/stopped from the tray menu.
//! On each click it photographs the active window (KWin ScreenShot2), marks the
//! click location, names the clicked element via AT-SPI; at the end it produces
//! an HTML/Markdown report. KDE Plasma / Wayland (first cut).

mod a11y;
mod annotate;
mod apply;
mod b64;
mod capture;
mod config;
mod cursor;
mod edit;
mod export_docx;
mod export_odt;
mod export_pdf;
mod export_rtf;
mod export_txt;
mod i18n;
mod icon;
mod input;
mod json;
mod keymap;
mod keys;
mod model;
mod notify;
mod report;
mod selftest;
mod session;
mod store;
mod tray;
mod zip;

use a11y::Atspi;
use anyhow::{Context, Result};
use capture::KdeCapturer;
use chrono::Local;
use config::Config;
use cursor::KwinCursor;
use input::{ClickSource, EvdevClickSource};
use ksni::blocking::TrayMethods;
use selftest::run_test_modes;
use session::{Session, TextEntry, capture_key_step, capture_step, finalize, output_base};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use store::write_session_json;
use tray::{Cmd, StepshotTray};

const USAGE: &str = "\
stepshot — step recorder for KDE/Wayland (tray app)

Usage: stepshot [OUTPUT_DIR]
       stepshot edit <SESSION_DIR>
       stepshot apply <SESSION_DIR> [EDITS_JSON]

Arguments:
  OUTPUT_DIR   base folder for sessions (default: ~/Pictures/stepshot)

Commands:
  edit         open the in-browser editor for a session (redact, edit text,
               delete/reorder steps); changes apply directly, no download
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
            return config::write_example();
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
        Some("edit") => {
            let Some(dir) = std::env::args().nth(2) else {
                eprintln!("usage: stepshot edit <SESSION_DIR>");
                std::process::exit(2);
            };
            return edit::run(std::path::Path::new(&dir));
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
    let keyboard = Arc::new(AtomicBool::new(config.capture.keyboard));
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();

    let handle = StepshotTray {
        tx: cmd_tx.clone(),
        recording: recording.clone(),
        paused: paused.clone(),
        steps: steps_count.clone(),
        keyboard: keyboard.clone(),
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

    // Keyboard steps (opt-in). The readers start the first time keyboard
    // capture is on and only report while `key_gate` is open (recording, not
    // paused, toggle on).
    let key_gate = Arc::new(AtomicBool::new(false));
    let (key_tx, key_rx) = mpsc::channel::<keys::KeyEvent>();
    let mut keys_started = false;
    if keyboard.load(Ordering::SeqCst) {
        keys_started = start_keyboard(&key_tx, &key_gate);
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
    // The text entry being typed right now (flushed into a step when it ends).
    let mut typing: Option<TextEntry> = None;
    // Shortcut labeler for the current layout, built lazily per session.
    let mut keymap: Option<keymap::Keymap> = None;
    let rec = Recorder {
        capturer: &capturer,
        cursor: &cursor,
        config: &config,
        steps_count: &steps_count,
    };
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
                    typing = None;
                    keymap = None; // re-read the layout for this session
                    drain(&key_rx);
                    steps_count.store(0, Ordering::SeqCst);
                    paused.store(false, Ordering::SeqCst);
                    recording.store(true, Ordering::SeqCst);
                    handle.update(|_| {});
                    // Don't record the clicks on the tray menu itself.
                    drain(&click_rx);
                    if let Some(c) = &notify_conn {
                        notify::notify(c, "stepshot", i18n::tr().notify_started, "stepshot");
                    }
                }
                Cmd::Stop => {
                    if let Some(mut s) = session.take() {
                        // Discard the gesture clicks that were still queued and
                        // trim the ones already captured as steps.
                        let pending = drain(&click_rx);
                        for dropped in session::trim_stop_gesture(&mut s.steps, pending) {
                            let _ = std::fs::remove_file(s.dir.join(&dropped.image_file));
                        }
                        drain(&key_rx);
                        rec.finish_typing(&mut s, &mut typing, None);
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
                        drain(&click_rx);
                        last_click = None;
                        if let Some(s) = session.as_mut() {
                            rec.finish_typing(s, &mut typing, None);
                        }
                        drain(&key_rx);
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
                Cmd::ToggleKeyboard => {
                    let on = !keyboard.load(Ordering::SeqCst);
                    keyboard.store(on, Ordering::SeqCst);
                    if on && !keys_started {
                        keys_started = start_keyboard(&key_tx, &key_gate);
                    }
                    if !on && let Some(s) = session.as_mut() {
                        rec.finish_typing(s, &mut typing, None);
                    }
                    handle.update(|_| {});
                }
                Cmd::OpenFolder => {
                    if let Some(d) = &last_dir {
                        let _ = std::process::Command::new("xdg-open").arg(d).spawn();
                    }
                }
                Cmd::EditLast => {
                    // Launch a separate `stepshot edit` process (the editor runs
                    // its own server loop and must not block the tray).
                    if let Some(d) = &last_dir {
                        match std::env::current_exe() {
                            Ok(exe) => {
                                let _ = std::process::Command::new(exe).arg("edit").arg(d).spawn();
                            }
                            Err(e) => eprintln!("[stepshot] could not locate own binary: {e}"),
                        }
                    } else if let Some(c) = &notify_conn {
                        notify::notify(c, "stepshot", i18n::tr().tt_ready, "stepshot");
                    }
                }
                Cmd::Quit | Cmd::Terminate => {
                    if let Some(mut s) = session.take() {
                        // Only a tray-initiated quit ends with tray clicks.
                        if cmd == Cmd::Quit {
                            let pending = drain(&click_rx);
                            for dropped in session::trim_stop_gesture(&mut s.steps, pending) {
                                let _ = std::fs::remove_file(s.dir.join(&dropped.image_file));
                            }
                        }
                        // A Ctrl+C that ended the run must not become a step.
                        drain(&key_rx);
                        rec.finish_typing(&mut s, &mut typing, None);
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
        key_gate.store(
            session.is_some() && !paused.load(Ordering::SeqCst) && keyboard.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );

        // Keyboard steps.
        if let Some(s) = session.as_mut() {
            while let Ok(ev) = key_rx.try_recv() {
                let km = keymap
                    .get_or_insert_with(|| keymap::Keymap::new(&keymap::LayoutNames::detect()));
                if rec.on_key(s, &mut typing, ev, km, &atspi) {
                    last_click = None; // a key step sits between two clicks
                }
            }
            let idle = Duration::from_millis(config.capture.typing_idle_ms);
            if typing.as_ref().is_some_and(|t| t.last.elapsed() >= idle) {
                rec.finish_typing(s, &mut typing, None);
                last_click = None;
            }
        }

        // Process clicks (with a timeout so commands are handled promptly).
        match click_rx.recv_timeout(Duration::from_millis(150)) {
            Ok(click) => {
                if let Some(s) = session.as_mut() {
                    // A click ends a text entry (focus moves on).
                    if rec.finish_typing(s, &mut typing, None) {
                        last_click = None;
                    }
                    let now = Instant::now();
                    // Skip while paused or when this button isn't recorded.
                    let drag = config.capture.drag_delta(&click);
                    if paused.load(Ordering::SeqCst) || !config.capture.records(click.button) {
                        // dropped — no step, last_click untouched
                    } else if drag.is_none()
                        && config
                            .capture
                            .is_double_click(last_click, click.button, now)
                    {
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
                            drag,
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
                                // A drag isn't a candidate for double-click merge.
                                last_click = if drag.is_some() {
                                    None
                                } else {
                                    Some((click.button, now))
                                };
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

/// Drain all queued events, returning how many were discarded.
fn drain<T>(rx: &mpsc::Receiver<T>) -> usize {
    let mut n = 0;
    while rx.try_recv().is_ok() {
        n += 1;
    }
    n
}

/// Start the keyboard readers; `false` (with a log line) if there's no keyboard.
fn start_keyboard(tx: &mpsc::Sender<keys::KeyEvent>, gate: &Arc<AtomicBool>) -> bool {
    match keys::start(tx.clone(), gate.clone()) {
        Ok(n) if n > 0 => true,
        Ok(_) => {
            eprintln!("[stepshot] no keyboard found — keyboard steps unavailable.");
            false
        }
        Err(e) => {
            eprintln!("[stepshot] keyboard capture unavailable: {e:#}");
            false
        }
    }
}

/// What the main loop needs to turn key events into steps.
struct Recorder<'a> {
    capturer: &'a KdeCapturer,
    cursor: &'a Option<KwinCursor>,
    config: &'a Config,
    steps_count: &'a AtomicUsize,
}

impl Recorder<'_> {
    /// Append a captured step and refresh the reports.
    fn push(&self, s: &mut Session, step: Result<model::Step>) -> bool {
        match step {
            Ok(step) => {
                s.steps.push(step);
                self.steps_count.store(s.steps.len(), Ordering::SeqCst);
                let _ = report::write_reports(&s.dir, &s.steps, &s.started, &self.config.export);
                write_session_json(s);
                true
            }
            Err(e) => {
                eprintln!("[stepshot] step {}: {e:#}", s.steps.len() + 1);
                false
            }
        }
    }

    /// Handle one key event; returns whether a step was recorded.
    fn on_key(
        &self,
        s: &mut Session,
        typing: &mut Option<TextEntry>,
        ev: keys::KeyEvent,
        km: &keymap::Keymap,
        atspi: &Option<Atspi>,
    ) -> bool {
        match ev {
            keys::KeyEvent::Text => {
                match typing {
                    Some(t) => t.last = Instant::now(),
                    // A new entry: remember the field now, while it has focus.
                    None => *typing = Some(TextEntry::begin(atspi)),
                }
                false
            }
            keys::KeyEvent::Edit { .. } if typing.is_some() => {
                if let Some(t) = typing {
                    t.last = Instant::now();
                }
                false
            }
            // Enter/Tab close the entry as “…, then pressed Enter”.
            ev if typing.is_some() && ev.ends_text_entry() => {
                self.finish_typing(s, typing, km.label_event(ev))
            }
            ev => {
                let Some(label) = km.label_event(ev) else {
                    return false; // lone arrow/Home/… — navigation, no step
                };
                let flushed = self.finish_typing(s, typing, None);
                let el = atspi.as_ref().and_then(|a| a.focused_element());
                let step = capture_key_step(
                    s.steps.len() + 1,
                    model::KeyKind::Press,
                    Some(label),
                    el.as_ref(),
                    &s.dir,
                    self.capturer,
                    self.cursor,
                );
                self.push(s, step) || flushed
            }
        }
    }

    /// Turn the running text entry (if any) into a step, optionally closed by
    /// `then` (Enter/Tab). Returns whether a step was recorded.
    fn finish_typing(
        &self,
        s: &mut Session,
        typing: &mut Option<TextEntry>,
        then: Option<String>,
    ) -> bool {
        let Some(t) = typing.take() else {
            return false;
        };
        let kind = if t.password {
            model::KeyKind::Password
        } else {
            model::KeyKind::Text
        };
        let step = capture_key_step(
            s.steps.len() + 1,
            kind,
            then,
            t.element.as_ref(),
            &s.dir,
            self.capturer,
            self.cursor,
        );
        self.push(s, step)
    }
}
