//! Windows backend (milestone 0.3).
//!
//! Mirrors the Linux backend behind the same [`crate::platform`] traits, using
//! native Win32:
//! - clicks: a low-level mouse hook (`WH_MOUSE_LL`) on its own message-loop thread;
//! - screenshot: `PrintWindow(PW_RENDERFULLCONTENT)` of the foreground window;
//! - cursor + geometry: `GetCursorPos` + `GetForegroundWindow`/`GetWindowRect`;
//! - element names: UI Automation `ElementFromPoint`;
//! - tray: `Shell_NotifyIcon` with a popup menu.
//!
//! Compiled only on Windows. Untested on a live machine yet — verified to
//! compile via the Windows CI job; functional testing is pending (alpha).

mod a11y;
mod capture;
mod cursor;
mod input;
mod tray;

use crate::platform::{ClickSource, CursorTracker, ElementResolver};
use crate::{Session, capture_step, finalize, output_base};
use anyhow::{Context, Result};
use chrono::Local;
use std::sync::mpsc;
use std::time::Duration;
use tray::{Cmd, Tray};

/// Windows entry point (called from `main`).
pub fn run() -> Result<()> {
    let capturer = capture::GdiCapturer::new();
    let cursor = cursor::WinCursor::new();
    let mut uia = a11y::UiaResolver::connect().ok();
    if uia.is_none() {
        eprintln!("[stepshot] UI Automation unavailable — element names disabled.");
    }

    let base = output_base()?;

    // Tray + its message pump live on this (main) thread.
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let mut tray = Tray::new(cmd_tx).context("could not create the tray icon")?;

    // Clicks arrive from the low-level hook thread.
    let (click_tx, click_rx) = mpsc::channel();
    input::WindowsClickSource.start(click_tx)?;

    eprintln!("stepshot is running in the tray — start/stop recording from the tray icon.");

    let mut session: Option<Session> = None;
    let mut last_dir = None;
    let mut run = true;

    while run {
        // Pump native tray/menu messages (non-blocking).
        tray.pump();

        // Control commands from the tray menu.
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
                    if let Some(u) = uia.as_mut() {
                        u.enable();
                    }
                    session = Some(Session {
                        dir: dir.clone(),
                        started: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                        steps: Vec::new(),
                    });
                    last_dir = Some(dir);
                    tray.set_recording(true, 0);
                    while click_rx.try_recv().is_ok() {}
                    tray.notify(crate::i18n::tr().notify_started);
                }
                Cmd::Stop => {
                    if let Some(s) = session.take() {
                        finalize(&s);
                        if let Some(u) = uia.as_ref() {
                            u.restore();
                        }
                        tray.set_recording(false, 0);
                        let msg = crate::i18n::tr()
                            .notify_stopped
                            .replace("{n}", &s.steps.len().to_string());
                        tray.notify(&msg);
                    }
                }
                Cmd::OpenFolder => {
                    if let Some(d) = &last_dir {
                        let _ = std::process::Command::new("explorer").arg(d).spawn();
                    }
                }
                Cmd::Quit => {
                    if let Some(s) = session.take() {
                        finalize(&s);
                        if let Some(u) = uia.as_ref() {
                            u.restore();
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

        // Process clicks (non-blocking; short sleep to avoid a busy spin).
        match click_rx.try_recv() {
            Ok(click) => {
                if let Some(s) = session.as_mut() {
                    let index = s.steps.len() + 1;
                    let res = capture_step(
                        index,
                        click.button,
                        &s.dir,
                        &capturer,
                        Some(&cursor as &dyn CursorTracker),
                        uia.as_ref().map(|u| u as &dyn ElementResolver),
                    );
                    match res {
                        Ok(step) => {
                            s.steps.push(step);
                            tray.set_recording(true, s.steps.len());
                            let _ = crate::report::write_reports(&s.dir, &s.steps, &s.started);
                        }
                        Err(e) => eprintln!("[stepshot] step {index}: {e:#}"),
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(15)),
            Err(mpsc::TryRecvError::Disconnected) => run = false,
        }
    }

    tray.remove();
    eprintln!("stepshot stopped.");
    Ok(())
}
