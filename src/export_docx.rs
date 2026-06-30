//! DOCX (Word) export with embedded screenshots.
//!
//! Pure-Rust via `docx-rs`. Unlike the PDF path this uses Word's own Unicode
//! fonts, so titles with emoji/CJK render fine — no sanitization needed.

use crate::model::Step;
use anyhow::{Context, Result};
use docx_rs::*;
use std::path::Path;

// Image sizing in EMU (English Metric Units): 914_400 per inch, 9_525 per pixel
// at 96 dpi. Keep screenshots within the printable area of a default page.
const EMU_PER_PX: f32 = 9_525.0;
const MAX_W_EMU: f32 = 6.0 * 914_400.0; // 6 inch content width
const MAX_H_EMU: f32 = 7.5 * 914_400.0; // 7.5 inch, leaves room for the heading

/// Writes `report.docx` into `dir`.
pub fn write(dir: &Path, steps: &[Step], started: &str) -> Result<()> {
    let t = crate::i18n::tr();
    let mut docx = Docx::new();

    // Title block.
    docx = docx.add_paragraph(para_run(Run::new().add_text("stepshot").bold().size(40)));
    docx = docx.add_paragraph(para_run(Run::new().add_text(t.report_heading).size(28)));
    docx = docx.add_paragraph(para_run(
        Run::new().add_text(t.report_started.replace("{x}", started)),
    ));
    docx = docx.add_paragraph(para_run(
        Run::new().add_text(t.report_total.replace("{n}", &steps.len().to_string())),
    ));
    docx = docx.add_paragraph(Paragraph::new()); // spacer

    for s in steps {
        let head = format!(
            "{}  —  {}",
            t.report_step.replace("{n}", &s.index.to_string()),
            s.time
        );
        docx = docx.add_paragraph(para_run(Run::new().add_text(head).bold().size(26)));
        docx = docx.add_paragraph(para_run(Run::new().add_text(s.describe())));

        let path = dir.join(&s.image_file);
        if let (Ok(bytes), Ok((w, h))) = (std::fs::read(&path), image::image_dimensions(&path)) {
            let (ew, eh) = fit_emu(w, h);
            let pic = Pic::new(&bytes).size(ew, eh);
            docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_image(pic)));
        }
        docx = docx.add_paragraph(Paragraph::new()); // spacer between steps
    }

    let file =
        std::fs::File::create(dir.join("report.docx")).context("could not create report.docx")?;
    docx.build()
        .pack(file)
        .context("could not write report.docx")?;
    Ok(())
}

fn para_run(run: Run) -> Paragraph {
    Paragraph::new().add_run(run)
}

/// Scale a pixel size down to fit the page, preserving aspect ratio. Returns EMU.
fn fit_emu(w_px: u32, h_px: u32) -> (u32, u32) {
    let (nat_w, nat_h) = (w_px as f32 * EMU_PER_PX, h_px as f32 * EMU_PER_PX);
    if nat_w <= 0.0 || nat_h <= 0.0 {
        return (1, 1);
    }
    let scale = 1.0_f32.min(MAX_W_EMU / nat_w).min(MAX_H_EMU / nat_h);
    ((nat_w * scale) as u32, (nat_h * scale) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Button;

    #[test]
    fn writes_a_nonempty_docx() {
        let dir = std::env::temp_dir().join(format!("stepshot-docx-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        ::image::RgbaImage::from_pixel(8, 6, ::image::Rgba([220, 30, 30, 255]))
            .save(dir.join("step-001.png"))
            .unwrap();
        let steps = vec![Step {
            index: 1,
            button: Button::Left,
            time: "12:00:00".into(),
            image_file: "step-001.png".into(),
            window_title: Some("Test Window 😀 — café".into()),
            element: Some("button “Save”".into()),
        }];
        write(&dir, &steps, "2026-01-01 12:00:00").unwrap();
        let f = dir.join("report.docx");
        let len = std::fs::metadata(&f).unwrap().len();
        assert!(len > 500, "docx suspiciously small: {len} bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fit_scales_down_large_images() {
        // 4000 px wide must shrink to <= 6 inch content width.
        let (w, _h) = fit_emu(4000, 3000);
        assert!(w as f32 <= MAX_W_EMU + 1.0);
        // A tiny image stays at native size (no upscaling).
        assert_eq!(fit_emu(10, 10), (10 * 9_525, 10 * 9_525));
    }
}
