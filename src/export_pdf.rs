//! PDF export (paginated, embedded screenshots).
//!
//! Pure-Rust via `printpdf` — no external runtime (no headless browser, no
//! `wkhtmltopdf`). Layout: a title page, then **one page per step** (header,
//! description, screenshot scaled to fit). Built-in Helvetica is used, so no
//! font file has to be bundled; text is therefore restricted to Latin-1 and
//! sanitized accordingly.

use crate::model::Step;
use anyhow::{Context, Result};
use printpdf::*;
use std::path::Path;

// A4 portrait, in millimetres.
const PAGE_W_MM: f32 = 210.0;
const PAGE_H_MM: f32 = 297.0;
const MARGIN_MM: f32 = 15.0;

const DARK: [f32; 3] = [0.11, 0.15, 0.22];
const MID: [f32; 3] = [0.40, 0.44, 0.50];
const ACCENT: [f32; 3] = [0.23, 0.51, 0.96];

/// Writes `report.pdf` into `dir`.
pub fn write(dir: &Path, steps: &[Step], started: &str) -> Result<()> {
    let mut doc = PdfDocument::new("stepshot");

    let mut pages = Vec::with_capacity(steps.len() + 1);
    pages.push(title_page(started, steps.len()));
    for s in steps {
        pages.push(step_page(&mut doc, dir, s));
    }

    let mut warnings = Vec::new();
    let bytes = doc
        .with_pages(pages)
        .save(&PdfSaveOptions::default(), &mut warnings);
    std::fs::write(dir.join("report.pdf"), bytes).context("could not write report.pdf")?;
    Ok(())
}

fn title_page(started: &str, count: usize) -> PdfPage {
    let t = crate::i18n::tr();
    let mut ops = Vec::new();
    text(
        &mut ops,
        MARGIN_MM,
        45.0,
        BuiltinFont::HelveticaBold,
        28.0,
        DARK,
        "stepshot",
    );
    text(
        &mut ops,
        MARGIN_MM,
        58.0,
        BuiltinFont::Helvetica,
        14.0,
        MID,
        t.report_heading,
    );
    text(
        &mut ops,
        MARGIN_MM,
        74.0,
        BuiltinFont::Helvetica,
        11.0,
        MID,
        &t.report_started.replace("{x}", started),
    );
    text(
        &mut ops,
        MARGIN_MM,
        82.0,
        BuiltinFont::Helvetica,
        11.0,
        MID,
        &t.report_total.replace("{n}", &count.to_string()),
    );
    PdfPage::new(Mm(PAGE_W_MM), Mm(PAGE_H_MM), ops)
}

fn step_page(doc: &mut PdfDocument, dir: &Path, s: &Step) -> PdfPage {
    let t = crate::i18n::tr();
    let mut ops = Vec::new();

    let head = format!(
        "{}  —  {}",
        t.report_step.replace("{n}", &s.index.to_string()),
        s.time
    );
    text(
        &mut ops,
        MARGIN_MM,
        22.0,
        BuiltinFont::HelveticaBold,
        15.0,
        ACCENT,
        &head,
    );

    // Description, wrapped to the content width.
    let mut y = 32.0;
    for line in wrap(&s.describe(), 92) {
        text(
            &mut ops,
            MARGIN_MM,
            y,
            BuiltinFont::Helvetica,
            11.0,
            DARK,
            &line,
        );
        y += 6.0;
    }

    // Screenshot below the text (best effort — a bad image just leaves the page text-only).
    let path = dir.join(&s.image_file);
    if let Ok(bytes) = std::fs::read(&path) {
        let mut warnings = Vec::new();
        if let Ok(raw) = RawImage::decode_from_bytes(&bytes, &mut warnings) {
            let (iw, ih) = (raw.width as f32, raw.height as f32);
            let id = doc.add_image(&raw);
            place_image(&mut ops, id, iw, ih, y + 4.0);
        }
    }

    PdfPage::new(Mm(PAGE_W_MM), Mm(PAGE_H_MM), ops)
}

/// Places an image, scaled to fit the content width and the remaining height.
fn place_image(ops: &mut Vec<Op>, id: XObjectId, iw_px: f32, ih_px: f32, top_mm: f32) {
    let content_w = pt(PAGE_W_MM - 2.0 * MARGIN_MM);
    let avail_h = pt(PAGE_H_MM - MARGIN_MM - top_mm);
    // Native size assuming 300 dpi (printpdf's image default).
    let nat_w = iw_px * 72.0 / 300.0;
    let nat_h = ih_px * 72.0 / 300.0;
    if nat_w <= 0.0 || nat_h <= 0.0 {
        return;
    }
    let scale = (content_w / nat_w).min(avail_h / nat_h);
    let draw_w = nat_w * scale;
    let draw_h = nat_h * scale;

    let x = pt(MARGIN_MM) + (content_w - draw_w) / 2.0;
    let top_y = pt(PAGE_H_MM - top_mm);
    let bottom_y = top_y - draw_h;

    ops.push(Op::UseXobject {
        id,
        transform: XObjectTransform {
            translate_x: Some(Pt(x)),
            translate_y: Some(Pt(bottom_y)),
            rotate: None,
            scale_x: Some(scale),
            scale_y: Some(scale),
            dpi: Some(300.0),
        },
    });
}

/// One line of text at `(x_mm, y_from_top_mm)` (top-left origin, like a reader expects).
fn text(
    ops: &mut Vec<Op>,
    x_mm: f32,
    y_top_mm: f32,
    font: BuiltinFont,
    size: f32,
    color: [f32; 3],
    s: &str,
) {
    let y_mm = PAGE_H_MM - y_top_mm; // convert to PDF's bottom-left origin
    ops.push(Op::StartTextSection);
    ops.push(Op::SetFillColor {
        col: Color::Rgb(Rgb::new(color[0], color[1], color[2], None)),
    });
    ops.push(Op::SetFont {
        font: PdfFontHandle::Builtin(font),
        size: Pt(size),
    });
    ops.push(Op::SetTextCursor {
        pos: Point {
            x: Pt(pt(x_mm)),
            y: Pt(pt(y_mm)),
        },
    });
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(sanitize(s))],
    });
    ops.push(Op::EndTextSection);
}

fn pt(mm: f32) -> f32 {
    mm * 2.834_646
}

/// Greedy word-wrap to roughly `max` characters per line.
fn wrap(s: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > max {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Built-in PDF fonts only cover Latin-1. Map common typographic characters to
/// ASCII and drop anything else so titles with emoji/CJK don't break rendering.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
            '\u{2018}' | '\u{2019}' | '\u{201A}' => '\'',
            '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            '\u{2026}' => '~', // ellipsis → placeholder (… is not Latin-1)
            '\u{00A0}' => ' ',
            c if (c as u32) <= 0xFF => c,
            _ => '?',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Button;

    #[test]
    fn writes_a_nonempty_pdf() {
        let dir = std::env::temp_dir().join(format!("stepshot-pdf-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A tiny screenshot to embed.
        ::image::RgbaImage::from_pixel(8, 6, ::image::Rgba([10, 120, 220, 255]))
            .save(dir.join("step-001.png"))
            .unwrap();
        let steps = vec![Step {
            index: 1,
            button: Button::Left,
            time: "12:00:00".into(),
            image_file: "step-001.png".into(),
            window_title: Some("Test “Window” — café".into()),
            element: Some("button “Save”".into()),
        }];
        write(&dir, &steps, "2026-01-01 12:00:00").unwrap();
        let pdf = dir.join("report.pdf");
        let len = std::fs::metadata(&pdf).unwrap().len();
        assert!(len > 500, "pdf suspiciously small: {len} bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_maps_typography() {
        assert_eq!(sanitize("“hi” — café"), "\"hi\" - caf\u{e9}");
        assert_eq!(sanitize("emoji \u{1F600}"), "emoji ?");
    }
}
