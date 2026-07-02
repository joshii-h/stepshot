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

/// Global cursor position and frame rect of the active window (screen coords),
/// plus the name of the output (monitor) the cursor sits on.
#[derive(Debug, Clone, Default)]
pub struct CursorInfo {
    pub x: i32,
    pub y: i32,
    pub frame_x: i32,
    pub frame_y: i32,
    pub frame_w: i32,
    pub frame_h: i32,
    /// The cursor is over a popup surface (context menu, dropdown — not a
    /// tooltip). Popups are separate Wayland surfaces and never show up in a
    /// window capture of their parent, so such clicks need a screen capture.
    pub in_popup: bool,
    /// KWin output name under the cursor (e.g. `DP-5`); empty if unknown.
    pub screen: String,
}

impl CursorInfo {
    /// Is the cursor inside the active window's frame rect? False also when
    /// there is no active window (frame is 0×0) — e.g. a click on the desktop
    /// or the panel, which never becomes the "active window".
    pub fn in_active_window(&self) -> bool {
        self.frame_w > 0
            && self.frame_h > 0
            && self.x >= self.frame_x
            && self.x < self.frame_x + self.frame_w
            && self.y >= self.frame_y
            && self.y < self.frame_y + self.frame_h
    }
}

/// D-Bus sink that the KWin script calls into.
struct Sink {
    tx: Sender<CursorInfo>,
}

#[interface(name = "org.stepshot.Sink")]
impl Sink {
    /// Called by the KWin script: "x,y,fx,fy,fw,fh,popup,screen" (seven ints +
    /// the output name under the cursor; the trailing name may be empty).
    fn report(&self, data: String) {
        let parts: Vec<&str> = data.split(',').collect();
        if parts.len() < 7 {
            return;
        }
        let v: Vec<i32> = parts[..7]
            .iter()
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if v.len() == 7 {
            let _ = self.tx.send(CursorInfo {
                x: v[0],
                y: v[1],
                frame_x: v[2],
                frame_y: v[3],
                frame_w: v[4],
                frame_h: v[5],
                in_popup: v[6] != 0,
                screen: parts
                    .get(7)
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default(),
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

// Report `workspace.cursorPos` + active-window frame + whether the cursor is
// over a popup surface + the output name under the cursor to our sink. The
// popup flag and the frame rect let the capture step decide when a window
// capture would miss what was clicked (context menu, panel, desktop); the
// screen name then selects the right monitor for the full-screen capture.
const KWIN_SCRIPT: &str = r#"(function(){
  var p = workspace.cursorPos;
  var w = workspace.activeWindow;
  var g = w ? w.frameGeometry : null;
  var inPopup = 0;
  var wins = workspace.windowList ? workspace.windowList() : [];
  for (var i = 0; i < wins.length; i++) {
    var win = wins[i];
    if (!win.popupWindow || win.tooltip || win.minimized) continue;
    var pg = win.frameGeometry;
    if (pg && p.x >= pg.x && p.x < pg.x + pg.width && p.y >= pg.y && p.y < pg.y + pg.height) {
      inPopup = 1; break;
    }
  }
  var sname = "";
  var scr = workspace.screens || [];
  for (var i = 0; i < scr.length; i++) {
    var sg = scr[i].geometry;
    if (sg && p.x >= sg.x && p.x < sg.x + sg.width && p.y >= sg.y && p.y < sg.y + sg.height) {
      sname = scr[i].name; break;
    }
  }
  var a = [p.x, p.y, g?g.x:0, g?g.y:0, g?g.width:0, g?g.height:0, inPopup].map(function(n){return Math.round(n);});
  callDBus("org.stepshot.Sink", "/sink", "org.stepshot.Sink", "Report", a.join(",") + "," + sname);
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

        let script_path = script_dir().join(format!("stepshot-cursor-{}.js", std::process::id()));
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

/// Where the KWin script file goes: `XDG_RUNTIME_DIR` (user-owned, mode 0700)
/// so no other local user can pre-create the predictably named path; `/tmp`
/// only as a fallback when the runtime dir is unavailable.
fn script_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

impl Drop for KwinCursor {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.script_path);
    }
}
