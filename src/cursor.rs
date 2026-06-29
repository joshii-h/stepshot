//! Global cursor position + window geometry on KDE/Wayland.
//!
//! Wayland does not expose the global pointer position to clients. The reliable
//! way to obtain it is from the compositor: we host a tiny D-Bus service
//! (`org.stepshot.Sink`) and, on each click, run a KWin script that reports
//! `workspace.cursorPos` and the window geometry back to us via `callDBus`.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use zbus::interface;

/// Global cursor position and frame rect of the active window (screen coords).
#[derive(Debug, Clone, Copy)]
pub struct CursorInfo {
    pub x: i32,
    pub y: i32,
    pub frame_x: i32,
    pub frame_y: i32,
    pub frame_w: i32,
    pub frame_h: i32,
}

/// D-Bus sink that the KWin script calls into.
struct Sink {
    tx: Sender<CursorInfo>,
}

#[interface(name = "org.stepshot.Sink")]
impl Sink {
    /// Called by the KWin script: "x,y,fx,fy,fw,fh" (all ints, CSV).
    fn report(&self, nums: String) {
        let v: Vec<i32> = nums
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if v.len() == 6 {
            let _ = self.tx.send(CursorInfo {
                x: v[0],
                y: v[1],
                frame_x: v[2],
                frame_y: v[3],
                frame_w: v[4],
                frame_h: v[5],
            });
        }
    }
}

/// Obtains the cursor position via a KWin script.
pub struct KwinCursor {
    conn: zbus::blocking::Connection,
    rx: Receiver<CursorInfo>,
    script_path: PathBuf,
    counter: AtomicI32,
}

// Report `workspace.cursorPos` + frame geometry to our sink.
const KWIN_SCRIPT: &str = r#"(function(){
  var p = workspace.cursorPos;
  var w = workspace.activeWindow;
  var g = w ? w.frameGeometry : null;
  var a = [p.x, p.y, g?g.x:0, g?g.y:0, g?g.width:0, g?g.height:0].map(function(n){return Math.round(n);});
  callDBus("org.stepshot.Sink", "/sink", "org.stepshot.Sink", "Report", a.join(","));
})();"#;

impl KwinCursor {
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let conn = zbus::blocking::connection::Builder::session()
            .context("session bus unreachable for cursor sink")?
            .name("org.stepshot.Sink")
            .context("bus name org.stepshot.Sink unavailable")?
            .serve_at("/sink", Sink { tx })
            .context("could not serve cursor sink")?
            .build()
            .context("could not start cursor sink")?;

        let script_path =
            std::env::temp_dir().join(format!("stepshot-cursor-{}.js", std::process::id()));
        std::fs::write(&script_path, KWIN_SCRIPT).context("could not write KWin script")?;

        Ok(Self {
            conn,
            rx,
            script_path,
            counter: AtomicI32::new(0),
        })
    }

    /// Loads + runs the KWin script and briefly waits for the callback.
    pub fn fetch(&self) -> Option<CursorInfo> {
        // Discard stale values.
        while self.rx.try_recv().is_ok() {}

        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let plugin = format!("stepshot{n}");

        let debug = std::env::var_os("STEPSHOT_DEBUG").is_some();

        let reply = match self.conn.call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "loadScript",
            &(self.script_path.to_string_lossy().as_ref(), plugin.as_str()),
        ) {
            Ok(r) => r,
            Err(e) => {
                if debug {
                    eprintln!("[stepshot] loadScript error: {e}");
                }
                return None;
            }
        };
        let id: i32 = reply.body().deserialize().ok()?;
        let obj = format!("/Scripting/Script{id}");
        if debug {
            eprintln!("[stepshot] loadScript id={id}, plugin={plugin}");
        }

        if let Err(e) = self.conn.call_method(
            Some("org.kde.KWin"),
            obj.as_str(),
            Some("org.kde.kwin.Script"),
            "run",
            &(),
        ) && debug
        {
            eprintln!("[stepshot] run error: {e}");
        }

        let info = self.rx.recv_timeout(Duration::from_millis(500)).ok();
        if debug {
            eprintln!("[stepshot] cursor info: {info:?}");
        }

        // Clean up so script instances don't accumulate.
        let _ = self.conn.call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "unloadScript",
            &(plugin.as_str(),),
        );

        info
    }
}

impl Drop for KwinCursor {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.script_path);
    }
}
