//! ODT (LibreOffice Writer) export with embedded screenshots.
//!
//! An ODT is a ZIP of XML: a `mimetype` marker, `content.xml`, a
//! `META-INF/manifest.xml`, and the images under `Pictures/`. Built with the
//! hand-rolled store-only [`crate::zip`] writer — no dependency.

use crate::model::Step;
use crate::report::html_escape;
use crate::zip::Zip;
use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::Path;

const MIME: &str = "application/vnd.oasis.opendocument.text";
/// Screenshots are 96 dpi; cap the display width at 16 cm to fit the page.
const CM_PER_PX: f32 = 2.54 / 96.0;
const MAX_W_CM: f32 = 16.0;

/// Writes `report.odt` into `dir`.
pub fn write(dir: &Path, steps: &[Step], started: &str) -> Result<()> {
    let mut zip = Zip::new();
    // The mimetype entry must be first and stored (our writer only stores).
    zip.add("mimetype", MIME.as_bytes());

    // Embed each screenshot under Pictures/ and remember its manifest line.
    let mut pictures = String::new();
    for s in steps {
        if s.image_file.is_empty() {
            continue;
        }
        if let Ok(bytes) = std::fs::read(dir.join(&s.image_file)) {
            let path = format!("Pictures/{}", s.image_file);
            zip.add(&path, &bytes);
            let _ = write!(
                pictures,
                "\n <manifest:file-entry manifest:full-path=\"{path}\" manifest:media-type=\"image/png\"/>"
            );
        }
    }

    zip.add("content.xml", content_xml(dir, steps, started).as_bytes());
    zip.add("META-INF/manifest.xml", manifest_xml(&pictures).as_bytes());

    std::fs::write(dir.join("report.odt"), zip.finish()).context("could not write report.odt")?;
    Ok(())
}

fn manifest_xml(pictures: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
 <manifest:file-entry manifest:full-path="/" manifest:media-type="{MIME}"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>{pictures}
</manifest:manifest>
"#
    )
}

fn content_xml(dir: &Path, steps: &[Step], started: &str) -> String {
    let t = crate::i18n::tr();
    let mut body = String::new();
    let _ = write!(
        body,
        "<text:h text:outline-level=\"1\">stepshot \u{2014} {}</text:h>",
        html_escape(t.report_heading)
    );
    let _ = write!(
        body,
        "<text:p>{}</text:p><text:p>{}</text:p>",
        html_escape(&t.report_started.replace("{x}", started)),
        html_escape(&t.report_total.replace("{n}", &steps.len().to_string()))
    );

    for s in steps {
        let label = t.report_step.replace("{n}", &s.index.to_string());
        let _ = write!(
            body,
            "<text:h text:outline-level=\"2\">{} \u{2014} {}</text:h>",
            html_escape(&label),
            html_escape(&s.time)
        );
        let _ = write!(body, "<text:p>{}</text:p>", html_escape(&s.describe()));
        if !s.image_file.is_empty()
            && let Some(frame) = image_frame(dir, &s.image_file)
        {
            let _ = write!(body, "<text:p>{frame}</text:p>");
        }
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" office:version="1.2">
 <office:body><office:text>{body}</office:text></office:body>
</office:document-content>
"#
    )
}

/// A `draw:frame` referencing the embedded picture, sized in cm.
fn image_frame(dir: &Path, name: &str) -> Option<String> {
    let (w, h) = image::image_dimensions(dir.join(name)).ok()?;
    let (mut wc, mut hc) = (w as f32 * CM_PER_PX, h as f32 * CM_PER_PX);
    if wc > MAX_W_CM {
        hc *= MAX_W_CM / wc;
        wc = MAX_W_CM;
    }
    Some(format!(
        "<draw:frame svg:width=\"{wc:.2}cm\" svg:height=\"{hc:.2}cm\"><draw:image xlink:href=\"Pictures/{name}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/></draw:frame>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Button;

    #[test]
    fn writes_a_nonempty_odt_zip() {
        let dir = std::env::temp_dir().join(format!("stepshot-odt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbaImage::from_pixel(8, 6, image::Rgba([30, 200, 60, 255]))
            .save(dir.join("step-001.png"))
            .unwrap();
        let steps = vec![Step {
            index: 1,
            button: Button::Left,
            time: "12:00:00".into(),
            image_file: "step-001.png".into(),
            window_title: Some("Café <x>".into()),
            process: None,
            element: Some("button “Save”".into()),
            element_box: None,
            description_override: None,
            is_screen: false,
            double: false,
            drag: false,
        }];
        write(&dir, &steps, "2026-01-01 12:00:00").unwrap();
        let bytes = std::fs::read(dir.join("report.odt")).unwrap();
        // Valid ZIP shell, and the mimetype is the first stored file.
        assert_eq!(&bytes[0..4], &0x0403_4b50u32.to_le_bytes());
        assert_eq!(&bytes[30..38], b"mimetype");
        assert!(bytes.len() > 200);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
