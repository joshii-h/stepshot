//! `stepshot apply <session-dir> [edits.json]` — the editing engine.
//!
//! Rebuilds a recorded session from its `session.json` source-of-truth, applying
//! the edits the in-report editor produced. With no `edits.json` it simply
//! regenerates every enabled export from the session as-is.
//!
//! Edits are one ordered list of entries — that single shape expresses delete
//! (omit an original), reorder (position in the list) and renumber (list index),
//! plus per-step description override and destructive redaction:
//!
//! ```json
//! {
//!   "steps": [
//!     { "ref": 2, "redact": [[x, y, w, h]] },
//!     { "ref": 1, "description": "Custom text" },
//!     { "ref": 3, "description": null },
//!     { "text": "A manual step", "image": "data:image/png;base64,…" }
//!   ]
//! }
//! ```
//!
//! A `ref` entry keeps an original step (index in `session.json`): `redact`
//! boxes are image-pixel coordinates, and `description` overrides the text
//! (`null` reverts to auto, absent leaves it). A `text` entry inserts a new
//! manual step; its `image` is optional (a filesystem path, or a `data:` URI as
//! the editor sends) — without one the step is text-only.

use crate::config::Config;
use crate::json::Json;
use crate::model::Step;
use crate::report;
use crate::session::Session;
use crate::store;
use anyhow::{Context, Result};
use image::RgbaImage;
use std::collections::HashMap;
use std::path::Path;

/// Destructively mosaic a rectangular region so the original detail is gone.
/// The block size scales with the region, so small boxes still get obscured.
pub fn pixelate(img: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32) {
    let (iw, ih) = img.dimensions();
    if x >= iw || y >= ih || w == 0 || h == 0 {
        return;
    }
    let x1 = (x + w).min(iw);
    let y1 = (y + h).min(ih);
    let block = ((x1 - x).min(y1 - y) / 6).clamp(6, 40);

    let mut by = y;
    while by < y1 {
        let mut bx = x;
        while bx < x1 {
            let cx1 = (bx + block).min(x1);
            let cy1 = (by + block).min(y1);
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for yy in by..cy1 {
                for xx in bx..cx1 {
                    let p = img.get_pixel(xx, yy).0;
                    r += p[0] as u64;
                    g += p[1] as u64;
                    b += p[2] as u64;
                    a += p[3] as u64;
                    n += 1;
                }
            }
            // The cell always has ≥1 pixel (cx1 > bx, cy1 > by); max(1) guards
            // the impossible zero without a division-by-zero branch.
            let n = n.max(1);
            let avg = image::Rgba([(r / n) as u8, (g / n) as u8, (b / n) as u8, (a / n) as u8]);
            for yy in by..cy1 {
                for xx in bx..cx1 {
                    img.put_pixel(xx, yy, avg);
                }
            }
            bx += block;
        }
        by += block;
    }
}

/// What to do with a step's display text.
#[derive(Debug, PartialEq)]
enum DescEdit {
    /// Leave whatever the session already has.
    Keep,
    /// Drop any override, reverting to the auto-generated text.
    Clear,
    /// Replace with this text.
    Set(String),
}

/// One entry in the ordered edit plan.
#[derive(Debug)]
enum EditEntry {
    /// Keep an original step (`ref`), optionally re-described and redacted.
    Keep {
        ref_index: usize,
        description: DescEdit,
        /// Image-pixel boxes to pixelate destructively.
        redactions: Vec<[u32; 4]>,
    },
    /// A brand-new manual step: required text, optional image (a filesystem
    /// path, or a `data:` URI as the editor sends it).
    Manual { text: String, image: Option<String> },
}

/// The full ordered edit plan.
#[derive(Debug)]
struct Edits {
    steps: Vec<EditEntry>,
}

impl Edits {
    /// The no-op plan: keep every step, in order, unchanged.
    fn identity(sess: &Session) -> Edits {
        Edits {
            steps: sess
                .steps
                .iter()
                .map(|s| EditEntry::Keep {
                    ref_index: s.index,
                    description: DescEdit::Keep,
                    redactions: Vec::new(),
                })
                .collect(),
        }
    }

    fn parse(text: &str) -> Result<Edits> {
        let j = Json::parse(text).map_err(|e| anyhow::anyhow!("edits.json: {e}"))?;
        let arr = j
            .get("steps")
            .and_then(Json::as_array)
            .context("edits.json: missing \"steps\" array")?;
        let mut steps = Vec::new();
        for e in arr {
            // A "ref" entry keeps an original; a "text" entry inserts a manual step.
            if let Some(r) = e.get("ref").and_then(Json::as_i64) {
                let description = match e.get("description") {
                    None => DescEdit::Keep,
                    Some(Json::Null) => DescEdit::Clear,
                    Some(v) => match v.as_str() {
                        Some(s) => DescEdit::Set(s.to_string()),
                        None => DescEdit::Keep,
                    },
                };
                let redactions = e
                    .get("redact")
                    .and_then(Json::as_array)
                    .map(|a| a.iter().filter_map(Json::as_u32x4).collect())
                    .unwrap_or_default();
                steps.push(EditEntry::Keep {
                    ref_index: r as usize,
                    description,
                    redactions,
                });
            } else if let Some(text) = e.get("text").and_then(Json::as_str) {
                let image = e.get("image").and_then(Json::as_str).map(str::to_string);
                steps.push(EditEntry::Manual {
                    text: text.to_string(),
                    image,
                });
            } else {
                anyhow::bail!("edits.json: an entry has neither \"ref\" nor \"text\"");
            }
        }
        Ok(Edits { steps })
    }
}

/// Decode a manual step's image: a `data:` URI (as the editor sends) or a
/// filesystem path. Normalized to RGBA.
fn load_manual_image(spec: &str) -> Result<RgbaImage> {
    if let Some(rest) = spec.strip_prefix("data:") {
        let payload = rest.split_once(',').map(|(_, p)| p).unwrap_or(rest);
        let bytes = crate::b64::base64_decode(payload).context("invalid base64 image data")?;
        Ok(image::load_from_memory(&bytes)
            .context("could not decode image data")?
            .to_rgba8())
    } else {
        Ok(image::open(spec)
            .with_context(|| format!("could not open image {spec}"))?
            .to_rgba8())
    }
}

/// CLI entry: apply `edits_path` (or, with `None`, just regenerate) and print
/// a summary.
pub fn run(session_dir: &Path, edits_path: Option<&Path>) -> Result<()> {
    let n = match edits_path {
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .with_context(|| format!("could not read {}", p.display()))?;
            apply_text(session_dir, &text)?
        }
        None => apply_identity(session_dir)?,
    };
    println!("Applied edits → {n} step(s) in {}", session_dir.display());
    Ok(())
}

/// Apply an in-memory `edits.json` (as produced by the editor's POST) to the
/// session, returning the resulting step count. Used by `stepshot edit`.
pub fn apply_text(session_dir: &Path, edits_text: &str) -> Result<usize> {
    let loaded = store::load_session(session_dir)?;
    crate::i18n::init_lang(&loaded.language);
    let edits = Edits::parse(edits_text)?;
    apply_plan(session_dir, &loaded.session, &edits)
}

/// Regenerate the session's exports from `session.json` unchanged.
fn apply_identity(session_dir: &Path) -> Result<usize> {
    let loaded = store::load_session(session_dir)?;
    crate::i18n::init_lang(&loaded.language);
    let edits = Edits::identity(&loaded.session);
    apply_plan(session_dir, &loaded.session, &edits)
}

/// The core: build the final steps + (redacted) images, swap them on disk,
/// rewrite `session.json`, and regenerate every enabled export.
fn apply_plan(session_dir: &Path, sess: &Session, edits: &Edits) -> Result<usize> {
    let config = Config::load();
    let by_index: HashMap<usize, &Step> = sess.steps.iter().map(|s| (s.index, s)).collect();

    // Build the final steps and their (redacted) images entirely in memory
    // first, so renumbering/reordering never clobbers a source file mid-flight.
    let mut final_steps = Vec::with_capacity(edits.steps.len());
    let mut images: Vec<(String, RgbaImage)> = Vec::with_capacity(edits.steps.len());
    for (pos, e) in edits.steps.iter().enumerate() {
        let new_index = pos + 1;
        let new_name = format!("step-{new_index:03}.png");

        let step = match e {
            EditEntry::Keep {
                ref_index,
                description,
                redactions,
            } => {
                let src = *by_index
                    .get(ref_index)
                    .with_context(|| format!("edits.json references unknown step {ref_index}"))?;
                let mut img = image::open(session_dir.join(&src.image_file))
                    .with_context(|| format!("could not open {}", src.image_file))?
                    .to_rgba8();
                for b in redactions {
                    pixelate(&mut img, b[0], b[1], b[2], b[3]);
                }
                images.push((new_name.clone(), img));

                let mut step = src.clone();
                step.index = new_index;
                step.image_file = new_name;
                match description {
                    DescEdit::Keep => {}
                    DescEdit::Clear => step.description_override = None,
                    DescEdit::Set(s) => step.description_override = Some(s.clone()),
                }
                step
            }
            EditEntry::Manual { text, image } => {
                // An image is optional; without one the step is text-only.
                let image_file = match image {
                    Some(spec) => {
                        let img = load_manual_image(spec)?;
                        images.push((new_name.clone(), img));
                        new_name
                    }
                    None => String::new(),
                };
                Step {
                    index: new_index,
                    image_file,
                    description_override: Some(text.clone()),
                    ..Step::default()
                }
            }
        };
        final_steps.push(step);
    }

    // Swap the images on disk: drop every old step-*.png, write the new set.
    remove_step_images(session_dir)?;
    for (name, img) in &images {
        img.save(session_dir.join(name))
            .with_context(|| format!("could not write {name}"))?;
    }

    let rebuilt = Session {
        dir: session_dir.to_path_buf(),
        started: sess.started.clone(),
        steps: final_steps,
    };
    store::write_session_json(&rebuilt);
    report::write_final(
        session_dir,
        &rebuilt.steps,
        &rebuilt.started,
        &config.export,
    )
    .context("could not rebuild reports")?;

    Ok(rebuilt.steps.len())
}

/// Delete every `step-*.png` in the folder (the new set is written afterwards).
fn remove_step_images(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("could not read {}", dir.display()))?
        .flatten()
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("step-") && name.ends_with(".png") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixelate_averages_into_uniform_blocks() {
        // A gradient so neighbouring pixels differ before redaction.
        let mut img = RgbaImage::from_fn(20, 20, |x, y| {
            image::Rgba([(x * 10) as u8, (y * 10) as u8, 0, 255])
        });
        pixelate(&mut img, 0, 0, 12, 12);
        // block size = min(12,12)/6 = 2 → clamped up to 6: cells of 6×6 px.
        assert_eq!(img.get_pixel(0, 0), img.get_pixel(5, 5)); // same block, now uniform
        assert_eq!(img.get_pixel(1, 4), img.get_pixel(4, 1));
        // Untouched outside the region.
        assert_eq!(*img.get_pixel(15, 15), image::Rgba([150, 150, 0, 255]));
    }

    #[test]
    fn pixelate_clamps_to_image_bounds() {
        let mut img = RgbaImage::from_pixel(10, 10, image::Rgba([1, 2, 3, 255]));
        // Region partly (and fully) off-image must not panic.
        pixelate(&mut img, 8, 8, 50, 50);
        pixelate(&mut img, 100, 100, 5, 5);
    }

    #[test]
    fn parses_reorder_delete_override_redact() {
        let e = Edits::parse(
            r#"{
              "steps": [
                { "ref": 2, "redact": [[1, 2, 3, 4], [5, 6, 7, 8]] },
                { "ref": 1, "description": "hi" },
                { "ref": 3, "description": null }
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(e.steps.len(), 3); // an omitted original = deleted
        match &e.steps[0] {
            EditEntry::Keep {
                ref_index,
                description,
                redactions,
            } => {
                assert_eq!(*ref_index, 2);
                assert_eq!(*redactions, vec![[1, 2, 3, 4], [5, 6, 7, 8]]);
                assert_eq!(*description, DescEdit::Keep);
            }
            _ => panic!("expected a Keep entry"),
        }
        assert!(
            matches!(&e.steps[1], EditEntry::Keep { description: DescEdit::Set(s), .. } if s == "hi")
        );
        assert!(matches!(
            &e.steps[2],
            EditEntry::Keep {
                description: DescEdit::Clear,
                ..
            }
        ));
    }

    #[test]
    fn parses_manual_step() {
        let e = Edits::parse(
            r#"{ "steps": [
                 { "ref": 1 },
                 { "text": "Open the terminal", "image": "data:image/png;base64,AAAA" },
                 { "text": "Type your name" }
               ] }"#,
        )
        .unwrap();
        assert_eq!(e.steps.len(), 3);
        assert!(matches!(&e.steps[0], EditEntry::Keep { ref_index: 1, .. }));
        assert!(
            matches!(&e.steps[1], EditEntry::Manual { text, image: Some(img) }
                if text == "Open the terminal" && img.starts_with("data:"))
        );
        assert!(
            matches!(&e.steps[2], EditEntry::Manual { text, image: None } if text == "Type your name")
        );
    }

    #[test]
    fn rejects_entry_with_neither_ref_nor_text() {
        assert!(Edits::parse(r#"{ "steps": [ { "redact": [] } ] }"#).is_err());
        assert!(Edits::parse(r#"{ "nope": [] }"#).is_err());
    }
}
