//! Window screenshot.
//!
//! KDE/Wayland: via the D-Bus interface `org.kde.KWin.ScreenShot2`.
//! KWin writes the raw image into a pipe file descriptor that we pass along
//! (FD passing) — exactly what `spectacle` does internally, just directly.
//!
//! The `WindowCapturer` trait abstracts the platform; a Windows backend
//! (`PrintWindow`) implements the same trait later.

use anyhow::{Context, Result};
use image::RgbaImage;
use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;
use zvariant::{Fd, OwnedValue, Value};

/// A captured window image plus optional context.
pub struct Capture {
    pub image: RgbaImage,
    pub window_title: Option<String>,
    /// Scale factor (HiDPI): image pixels = logical coords * scale.
    pub scale: f64,
}

/// Backend that photographs the currently active window.
pub trait WindowCapturer {
    fn capture_active_window(&self) -> Result<Capture>;
}

/// KWin ScreenShot2 backend (KDE Plasma, Wayland & X11).
pub struct KdeCapturer {
    conn: zbus::blocking::Connection,
    debug: bool,
}

impl KdeCapturer {
    pub fn connect() -> Result<Self> {
        let conn = zbus::blocking::Connection::session()
            .context("session D-Bus unreachable (is a KDE session running?)")?;
        Ok(Self {
            conn,
            debug: std::env::var_os("STEPSHOT_DEBUG").is_some(),
        })
    }
}

impl WindowCapturer for KdeCapturer {
    fn capture_active_window(&self) -> Result<Capture> {
        // Pipe: KWin gets the write end, we read the image from the read end.
        let (mut reader, writer) = os_pipe::pipe().context("could not create pipe")?;

        // include-cursor: KWin renders the real mouse cursor into the image →
        // the click is visible (exactly where it happened).
        let mut options: HashMap<String, Value> = HashMap::new();
        options.insert("include-cursor".into(), Value::Bool(true));
        options.insert("include-decoration".into(), Value::Bool(true));

        let fd = Fd::from(writer.as_fd());

        let reply = self
            .conn
            .call_method(
                Some("org.kde.KWin"),
                "/org/kde/KWin/ScreenShot2",
                Some("org.kde.KWin.ScreenShot2"),
                "CaptureActiveWindow",
                &(options, fd),
            )
            .context("CaptureActiveWindow failed (KWin may gate this interface)")?;

        // Close our write end, otherwise the read never reaches EOF.
        drop(writer);

        let results: HashMap<String, OwnedValue> = reply
            .body()
            .deserialize()
            .context("could not read ScreenShot2 reply")?;

        if self.debug {
            let keys: Vec<&String> = results.keys().collect();
            eprintln!("[stepshot] ScreenShot2 results keys: {keys:?}");
        }

        let width = get_i64(&results, "width").context("no 'width' in reply")? as u32;
        let height = get_i64(&results, "height").context("no 'height' in reply")? as u32;
        let stride = get_i64(&results, "stride").context("no 'stride' in reply")? as usize;
        let format = get_i64(&results, "format").unwrap_or(6); // 6 = ARGB32_Premultiplied

        // Read the raw bytes from the pipe (height * stride).
        let mut raw = Vec::with_capacity(stride.saturating_mul(height as usize));
        reader
            .read_to_end(&mut raw)
            .context("could not read image data")?;

        let image = decode_qimage(&raw, width, height, stride, format)
            .context("could not decode raw image")?;

        // ScreenShot2 reports the windowId (UUID); we use it to get the title.
        let window_title = get_string(&results, "windowId").and_then(|id| self.window_caption(&id));

        let scale = get_f64(&results, "scale").unwrap_or(1.0);

        Ok(Capture {
            image,
            window_title,
            scale,
        })
    }
}

impl KdeCapturer {
    /// Resolve the window title for a UUID via `org.kde.KWin.getWindowInfo`.
    fn window_caption(&self, window_id: &str) -> Option<String> {
        let reply = self
            .conn
            .call_method(
                Some("org.kde.KWin"),
                "/KWin",
                Some("org.kde.KWin"),
                "getWindowInfo",
                &(window_id,),
            )
            .ok()?;
        let info: HashMap<String, OwnedValue> = reply.body().deserialize().ok()?;
        let caption = get_string(&info, "caption")?;
        if caption.is_empty() {
            None
        } else {
            Some(caption)
        }
    }
}

/// Converts KWin's raw buffer into an RGBA image.
///
/// KWin usually delivers QImage::Format_ARGB32_Premultiplied (6): in memory,
/// little-endian, that is B,G,R,A with premultiplied alpha. We swap B/R and undo
/// the premultiplication so shadows/rounded corners stay clean.
fn decode_qimage(
    raw: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    format: i64,
) -> Result<RgbaImage> {
    anyhow::ensure!(
        width > 0 && height > 0,
        "invalid image size {width}x{height}"
    );
    anyhow::ensure!(
        stride >= width as usize * 4,
        "stride {stride} smaller than row width"
    );
    anyhow::ensure!(
        raw.len() >= stride * height as usize,
        "not enough image data: {} < {}",
        raw.len(),
        stride * height as usize
    );

    let premultiplied = format == 6 || format == 7; // *_Premultiplied
    let mut img = RgbaImage::new(width, height);

    for y in 0..height as usize {
        let row = &raw[y * stride..y * stride + width as usize * 4];
        for x in 0..width as usize {
            let p = &row[x * 4..x * 4 + 4];
            let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
            let (r, g, b) = if premultiplied && a > 0 && a < 255 {
                let un = |c: u8| ((c as u16 * 255 + a as u16 / 2) / a as u16).min(255) as u8;
                (un(r), un(g), un(b))
            } else {
                (r, g, b)
            };
            img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, a]));
        }
    }
    Ok(img)
}

/// Pull an integer value from the result dict (Qt mixes i32/u32/i64).
fn get_i64(map: &HashMap<String, OwnedValue>, key: &str) -> Option<i64> {
    let v = map.get(key)?;
    match &**v {
        Value::U8(n) => Some(*n as i64),
        Value::I16(n) => Some(*n as i64),
        Value::U16(n) => Some(*n as i64),
        Value::I32(n) => Some(*n as i64),
        Value::U32(n) => Some(*n as i64),
        Value::I64(n) => Some(*n),
        Value::U64(n) => Some(*n as i64),
        _ => None,
    }
}

fn get_string(map: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let v = map.get(key)?;
    match &**v {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn get_f64(map: &HashMap<String, OwnedValue>, key: &str) -> Option<f64> {
    let v = map.get(key)?;
    match &**v {
        Value::F64(n) => Some(*n),
        Value::I32(n) => Some(*n as f64),
        Value::U32(n) => Some(*n as f64),
        _ => None,
    }
}
