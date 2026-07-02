//! Window screenshot.
//!
//! KDE/Wayland: via the D-Bus interface `org.kde.KWin.ScreenShot2`.
//! KWin writes the raw image into a pipe file descriptor that we pass along
//! (FD passing) — exactly what `spectacle` does internally, just directly.
//!
//! Implements the [`WindowCapturer`] platform trait; the Windows backend
//! (`PrintWindow`) implements the same trait in `win::capture`.

use crate::platform::{Capture, CursorInfo, WindowCapturer};
use anyhow::{Context, Result};
use image::RgbaImage;
use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;
use std::sync::mpsc;
use std::time::Duration;
use zvariant::{Fd, OwnedValue, Value};

/// Maximum time to wait for KWin to stream the image into the pipe. Normally
/// this is near-instant (local pipe); the deadline only guards against a
/// compositor that never closes its write end, which would otherwise hang the
/// main loop forever.
const READ_DEADLINE: Duration = Duration::from_secs(10);

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
        self.capture_via("CaptureActiveWindow", None)
    }

    /// Captures the output under the cursor by name, or the active screen when
    /// the output name is unknown. KWin renders the real cursor into the image
    /// (`include-cursor`), so the click location is visible without a marker.
    fn capture_screen_under_cursor(&self, ci: Option<&CursorInfo>) -> Result<Capture> {
        match ci {
            Some(c) if !c.screen.is_empty() => self.capture_via("CaptureScreen", Some(&c.screen)),
            _ => self.capture_via("CaptureActiveScreen", None),
        }
    }
}

impl KdeCapturer {
    /// Runs one ScreenShot2 capture method and decodes the result. `screen` is
    /// the leading output-name argument for `CaptureScreen`; `None` for the
    /// `CaptureActiveWindow` / `CaptureActiveScreen` variants.
    fn capture_via(&self, method: &str, screen: Option<&str>) -> Result<Capture> {
        // Pipe: KWin gets the write end, we read the image from the read end.
        let (mut reader, writer) = os_pipe::pipe().context("could not create pipe")?;

        // include-cursor: KWin renders the real mouse cursor into the image →
        // the click is visible (exactly where it happened).
        let mut options: HashMap<String, Value> = HashMap::new();
        options.insert("include-cursor".into(), Value::Bool(true));
        options.insert("include-decoration".into(), Value::Bool(true));

        let fd = Fd::from(writer.as_fd());

        // CaptureScreen takes a leading output-name argument; the others don't.
        let path = "/org/kde/KWin/ScreenShot2";
        let iface = Some("org.kde.KWin.ScreenShot2");
        let call = match screen {
            Some(name) => self.conn.call_method(
                Some("org.kde.KWin"),
                path,
                iface,
                method,
                &(name, options, fd),
            ),
            None => {
                self.conn
                    .call_method(Some("org.kde.KWin"), path, iface, method, &(options, fd))
            }
        };
        let reply =
            call.with_context(|| format!("{method} failed (KWin may gate this interface)"))?;

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

        // Read the raw bytes from the pipe (height * stride) on a helper thread
        // with a deadline — like the AT-SPI queries, a stuck read must not hang
        // the recorder. On timeout the reader thread stays blocked in the
        // background until KWin eventually closes the fd.
        let expected = stride.saturating_mul(height as usize);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut raw = Vec::with_capacity(expected);
            let res = reader.read_to_end(&mut raw).map(|_| raw);
            let _ = tx.send(res);
        });
        let raw = rx
            .recv_timeout(READ_DEADLINE)
            .context("timed out waiting for image data from KWin")?
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
            is_screen: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One BGRA pixel, optionally premultiplied, in a row padded to `stride`.
    fn buf(pixel: [u8; 4], stride: usize) -> Vec<u8> {
        let mut raw = vec![0u8; stride];
        raw[..4].copy_from_slice(&pixel);
        raw
    }

    #[test]
    fn decode_swaps_bgra_to_rgba() {
        // B=10, G=20, R=30, A=255 (format 5 = ARGB32, not premultiplied)
        let img = decode_qimage(&buf([10, 20, 30, 255], 4), 1, 1, 4, 5).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [30, 20, 10, 255]);
    }

    #[test]
    fn decode_unpremultiplies_format_6() {
        // Premultiplied with a=128: stored channel 64 ≈ original 127.
        let img = decode_qimage(&buf([64, 64, 64, 128], 4), 1, 1, 4, 6).unwrap();
        let p = img.get_pixel(0, 0).0;
        assert_eq!(p[3], 128);
        for c in &p[..3] {
            assert!((126..=129).contains(c), "channel {c} should be ≈127");
        }
    }

    #[test]
    fn decode_respects_stride_padding() {
        // stride 8 for a 1-px row: the pixel must come from the row start.
        let img = decode_qimage(&buf([0, 0, 200, 255], 8), 1, 1, 8, 5).unwrap();
        assert_eq!(img.get_pixel(0, 0).0[0], 200);
    }

    #[test]
    fn decode_rejects_short_buffers_and_zero_sizes() {
        assert!(decode_qimage(&[0u8; 4], 2, 2, 8, 5).is_err()); // too little data
        assert!(decode_qimage(&[], 0, 1, 4, 5).is_err()); // zero width
        assert!(decode_qimage(&[0u8; 4], 1, 1, 2, 5).is_err()); // stride < row
    }
}
