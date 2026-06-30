//! Builds a document (HTML + Markdown) from the captured steps.
//!
//! Two HTML variants:
//! - **live** (during recording, after each step): images as file references —
//!   fast to write, serves as a safety net.
//! - **final** (on stop): images **embedded** as base64 data URIs → a single,
//!   self-contained file you can send.

use crate::model::Step;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Live variant (file references) — after each step.
pub fn write_reports(dir: &Path, steps: &[Step], started: &str) -> Result<()> {
    fs::write(
        dir.join("report.html"),
        render_html(steps, started, dir, false),
    )
    .context("could not write report.html")?;
    fs::write(dir.join("report.md"), render_markdown(steps, started))
        .context("could not write report.md")?;
    Ok(())
}

/// Final variant — when recording stops. Writes the self-contained HTML and
/// Markdown, then (best effort) the PDF and DOCX exports. A failing export does
/// not lose the report: HTML/Markdown are written first and their errors are the
/// only ones propagated; PDF/DOCX failures are logged and swallowed.
pub fn write_final(dir: &Path, steps: &[Step], started: &str) -> Result<()> {
    fs::write(
        dir.join("report.html"),
        render_html(steps, started, dir, true),
    )
    .context("could not write report.html (final)")?;
    fs::write(dir.join("report.md"), render_markdown(steps, started))
        .context("could not write report.md")?;

    if let Err(e) = crate::export_pdf::write(dir, steps, started) {
        eprintln!("[stepshot] PDF export failed: {e:#}");
    }
    if let Err(e) = crate::export_docx::write(dir, steps, started) {
        eprintln!("[stepshot] DOCX export failed: {e:#}");
    }
    Ok(())
}

fn render_markdown(steps: &[Step], started: &str) -> String {
    let t = crate::i18n::tr();
    let mut out = String::new();
    out.push_str(&format!(
        "# {}\n\n{}\n\n",
        t.report_heading,
        t.report_started.replace("{x}", started)
    ));
    out.push_str(&format!(
        "{}\n\n",
        t.report_total.replace("{n}", &steps.len().to_string())
    ));
    for s in steps {
        let step_label = t.report_step.replace("{n}", &s.index.to_string());
        out.push_str(&format!(
            "## {step_label} — {}\n\n*{}*\n\n![{step_label}]({})\n\n",
            s.time,
            s.describe(),
            s.image_file
        ));
    }
    out
}

/// `embed=true` inlines the images as base64 data URIs (self-contained).
fn render_html(steps: &[Step], started: &str, dir: &Path, embed: bool) -> String {
    let t = crate::i18n::tr();
    let mut cards = String::new();
    for s in steps {
        let src = if embed {
            match fs::read(dir.join(&s.image_file)) {
                Ok(bytes) => format!("data:image/png;base64,{}", base64(&bytes)),
                Err(_) => html_escape(&s.image_file), // fallback: file reference
            }
        } else {
            html_escape(&s.image_file)
        };
        cards.push_str(&format!(
            r#"  <section class="step">
    <div class="head"><span class="num">{n}</span>
      <div><p class="desc">{desc}</p><p class="time">{time}</p></div>
    </div>
    <img src="{src}" alt="{alt}" loading="lazy">
  </section>
"#,
            n = s.index,
            alt = html_escape(&t.report_step.replace("{n}", &s.index.to_string())),
            desc = html_escape(&s.describe()),
            time = html_escape(&s.time),
            src = src,
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="{html_lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>stepshot — {heading}</title>
<style>
  :root {{ color-scheme: light dark; }}
  body {{ font-family: system-ui, sans-serif; max-width: 980px; margin: 2rem auto; padding: 0 1rem; line-height: 1.5; }}
  header {{ border-bottom: 2px solid #8884; padding-bottom: .75rem; margin-bottom: 1.5rem; }}
  h1 {{ margin: 0; font-size: 1.6rem; }}
  .meta {{ color: #8888; font-size: .9rem; }}
  .step {{ margin: 0 0 2.5rem; }}
  .head {{ display: flex; align-items: center; gap: .9rem; margin-bottom: .6rem; }}
  .num {{ flex: 0 0 auto; width: 2rem; height: 2rem; border-radius: 50%; background: #3b82f6;
          color: #fff; display: grid; place-items: center; font-weight: 700; }}
  .desc {{ margin: 0; font-weight: 600; }}
  .time {{ margin: 0; color: #8888; font-size: .85rem; }}
  img {{ max-width: 100%; height: auto; border: 1px solid #8884; border-radius: 8px;
         box-shadow: 0 2px 12px #0003; }}
</style>
</head>
<body>
<header>
  <h1>{heading}</h1>
  <p class="meta">{started_line} · {count} {steps_word}{embed_note}</p>
</header>
{cards}</body>
</html>
"#,
        html_lang = t.html_lang,
        heading = html_escape(t.report_heading),
        started_line = html_escape(&t.report_started.replace("{x}", started)),
        count = steps.len(),
        steps_word = html_escape(t.report_steps_word),
        embed_note = if embed {
            format!(" · {}", html_escape(t.report_self_contained))
        } else {
            String::new()
        },
        cards = cards,
    )
}

/// Minimal base64 encoder (standard alphabet, dependency-free).
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
