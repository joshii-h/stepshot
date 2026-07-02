//! Env-driven debug / self-test modes, checked once at startup. Each returns
//! `true` when it handled the run, so `main` exits instead of entering the loop.

use crate::a11y::Atspi;
use crate::capture::KdeCapturer;
use crate::config::Config;
use crate::cursor::KwinCursor;
use crate::icon;
use crate::model::Button;
use crate::session::{Session, capture_step, finalize, output_base};
use anyhow::Result;
use chrono::Local;
use std::time::Duration;

/// Debug/self-test modes (env-driven). Returns true if handled.
pub fn run_test_modes(
    capturer: &KdeCapturer,
    cursor: &Option<KwinCursor>,
    atspi: &mut Option<Atspi>,
    config: &Config,
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
        let dir = output_base(config)?.join(format!("oneshot-{}", Local::now().format("%H-%M-%S")));
        std::fs::create_dir_all(&dir)?;
        if let Some(a) = atspi.as_mut() {
            a.enable();
        }
        let started = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let step = capture_step(
            1,
            Button::Left,
            &dir,
            capturer,
            cursor,
            atspi,
            &config.marker,
        )?;
        println!("Oneshot → {}", step.describe());
        // Route through a real Session so this also writes session.json — the
        // full capture→serialize→report pipeline in one shot.
        let session = Session {
            dir: dir.clone(),
            started,
            steps: vec![step],
        };
        finalize(&session, &config.export);
        if let Some(a) = atspi.as_ref() {
            a.restore();
        }
        println!("Report: {}", dir.join("report.html").display());
        return Ok(true);
    }
    Ok(false)
}
