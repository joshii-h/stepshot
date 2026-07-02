//! RTF export (Word / WordPad) with embedded screenshots.
//!
//! RTF is plain text: the document is control words plus escaped text, and each
//! screenshot is embedded as a `\pngblip` picture with hex-encoded PNG bytes.
//! Pure-Rust, no dependency — same spirit as the other hand-rolled writers.

use crate::model::Step;
use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::Path;

/// 1 inch = 1440 twips; screenshots are treated as 96 dpi (15 twips per pixel).
const TWIPS_PER_PX: f32 = 15.0;
/// Cap the display width at 6 inches so images fit the page.
const MAX_W_TWIPS: f32 = 6.0 * 1440.0;

/// Writes `report.rtf` into `dir`.
pub fn write(dir: &Path, steps: &[Step], started: &str) -> Result<()> {
    let t = crate::i18n::tr();
    let mut rtf = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0 Segoe UI;}}\n");

    let _ = writeln!(
        rtf,
        "\\fs40\\b stepshot\\b0  {}\\par",
        esc(t.report_heading)
    );
    let _ = writeln!(
        rtf,
        "\\fs20 {}\\line {}\\par",
        esc(&t.report_started.replace("{x}", started)),
        esc(&t.report_total.replace("{n}", &steps.len().to_string()))
    );

    for s in steps {
        let label = t.report_step.replace("{n}", &s.index.to_string());
        let _ = writeln!(
            rtf,
            "\\par\\fs28\\b {} \\endash  {}\\b0\\par",
            esc(&label),
            esc(&s.time)
        );
        let _ = writeln!(rtf, "\\fs22 {}\\par", esc(&s.describe()));
        if !s.image_file.is_empty()
            && let Some(pic) = image_rtf(dir, &s.image_file)
        {
            rtf.push_str(&pic);
            rtf.push_str("\\par\n");
        }
    }

    rtf.push_str("}\n");
    std::fs::write(dir.join("report.rtf"), rtf).context("could not write report.rtf")?;
    Ok(())
}

/// Escape text for RTF: control chars `\{}` and any non-ASCII as `\uN?`.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '{' => o.push_str("\\{"),
            '}' => o.push_str("\\}"),
            c if (c as u32) < 0x80 => o.push(c),
            c => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    // RTF wants a signed 16-bit code unit, plus an ASCII fallback.
                    let _ = write!(o, "\\u{}?", *u as i16);
                }
            }
        }
    }
    o
}

/// Build a `{\pict …}` group embedding the PNG as hex, scaled to fit the page.
fn image_rtf(dir: &Path, name: &str) -> Option<String> {
    let path = dir.join(name);
    let bytes = std::fs::read(&path).ok()?;
    let (w, h) = image::image_dimensions(&path).ok()?;
    let (tw, th) = (w as f32 * TWIPS_PER_PX, h as f32 * TWIPS_PER_PX);
    let scale = (MAX_W_TWIPS / tw).min(1.0);
    let (gw, gh) = ((tw * scale) as u32, (th * scale) as u32);

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2 + 64);
    let _ = writeln!(s, "{{\\pict\\pngblip\\picwgoal{gw}\\pichgoal{gh}");
    for b in &bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s.push('}');
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_braces_and_unicode() {
        assert_eq!(esc("a{b}c\\"), "a\\{b\\}c\\\\");
        assert_eq!(esc("é"), "\\u233?"); // U+00E9
    }

    #[test]
    fn writes_a_nonempty_rtf_with_picture() {
        let dir = std::env::temp_dir().join(format!("stepshot-rtf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbaImage::from_pixel(6, 4, image::Rgba([10, 20, 200, 255]))
            .save(dir.join("step-001.png"))
            .unwrap();
        let steps = vec![Step {
            index: 1,
            time: "12:00:00".into(),
            image_file: "step-001.png".into(),
            window_title: Some("Café".into()),
            element: Some("button “Save”".into()),
            ..Step::default()
        }];
        write(&dir, &steps, "2026-01-01 12:00:00").unwrap();
        let rtf = std::fs::read_to_string(dir.join("report.rtf")).unwrap();
        assert!(rtf.starts_with("{\\rtf1"));
        assert!(rtf.contains("\\pngblip"));
        assert!(rtf.trim_end().ends_with('}'));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
