//! Plain-text export — numbered steps, screenshots referenced by filename.

use crate::model::Step;
use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::path::Path;

/// Writes `report.txt` into `dir`.
pub fn write(dir: &Path, steps: &[Step], started: &str) -> Result<()> {
    let t = crate::i18n::tr();
    let mut out = String::new();
    let _ = writeln!(out, "stepshot — {}", t.report_heading);
    let _ = writeln!(out, "{}", t.report_started.replace("{x}", started));
    let _ = writeln!(
        out,
        "{}\n",
        t.report_total.replace("{n}", &steps.len().to_string())
    );

    for s in steps {
        let label = t.report_step.replace("{n}", &s.index.to_string());
        let _ = writeln!(out, "{label} — {}", s.time);
        let _ = writeln!(out, "  {}", s.describe());
        if !s.image_file.is_empty() {
            let _ = writeln!(out, "  [{}]", s.image_file);
        }
        out.push('\n');
    }

    std::fs::write(dir.join("report.txt"), out).context("could not write report.txt")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(index: usize, image: &str, desc_override: &str) -> Step {
        Step {
            index,
            time: "12:00:00".into(),
            image_file: image.into(),
            window_title: Some("Win".into()),
            description_override: (!desc_override.is_empty()).then(|| desc_override.to_string()),
            ..Step::default()
        }
    }

    #[test]
    fn writes_numbered_steps_and_skips_missing_image() {
        let dir = std::env::temp_dir().join(format!("stepshot-txt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let steps = vec![step(1, "step-001.png", ""), step(2, "", "A manual note")];
        write(&dir, &steps, "2026-01-01 12:00:00").unwrap();
        let text = std::fs::read_to_string(dir.join("report.txt")).unwrap();
        assert!(text.contains("[step-001.png]"));
        assert!(text.contains("A manual note"));
        assert!(
            !text.contains("[]"),
            "imageless step must not print a bracket"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
