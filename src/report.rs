//! Builds a document (HTML + Markdown) from the captured steps.
//!
//! Two HTML variants:
//! - **live** (during recording, after each step): images as file references —
//!   fast to write, serves as a safety net.
//! - **final** (on stop): images **embedded** as base64 data URIs → a single,
//!   self-contained file you can send.

use crate::config::{ExportConfig, Format};
use crate::model::Step;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Live variant (file references) — after each step. Only the selected
/// file-based formats (HTML/Markdown) are written incrementally; PDF/DOCX are
/// final-only. If neither HTML nor Markdown is enabled, this is a no-op.
pub fn write_reports(
    dir: &Path,
    steps: &[Step],
    started: &str,
    export: &ExportConfig,
) -> Result<()> {
    if export.has(Format::Html) {
        fs::write(
            dir.join("report.html"),
            render_html(steps, started, dir, false),
        )
        .context("could not write report.html")?;
    }
    if export.has(Format::Md) {
        fs::write(dir.join("report.md"), render_markdown(steps, started))
            .context("could not write report.md")?;
    }
    Ok(())
}

/// Final variant — when recording stops. Writes each selected format: the
/// self-contained HTML and Markdown first (their errors propagate), then the
/// PDF and DOCX exports best-effort (failures are logged and swallowed so a bad
/// export never loses the whole report).
pub fn write_final(dir: &Path, steps: &[Step], started: &str, export: &ExportConfig) -> Result<()> {
    if export.has(Format::Html) {
        fs::write(
            dir.join("report.html"),
            render_html(steps, started, dir, true),
        )
        .context("could not write report.html (final)")?;
    }
    if export.has(Format::Md) {
        fs::write(dir.join("report.md"), render_markdown(steps, started))
            .context("could not write report.md")?;
    }
    if export.has(Format::Pdf)
        && let Err(e) = crate::export_pdf::write(dir, steps, started)
    {
        eprintln!("[stepshot] PDF export failed: {e:#}");
    }
    if export.has(Format::Docx)
        && let Err(e) = crate::export_docx::write(dir, steps, started)
    {
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
        let app = match &s.process {
            Some(p) if !p.is_empty() => format!(" · `{}`", md_escape(p)),
            _ => String::new(),
        };
        let image = if s.image_file.is_empty() {
            String::new()
        } else {
            format!("![{step_label}]({})\n\n", s.image_file)
        };
        out.push_str(&format!(
            "## {step_label} — {}{app}\n\n*{}*\n\n{image}",
            s.time,
            md_escape(&s.describe()),
        ));
    }
    out
}

/// Escape Markdown syntax in free text (window titles, element names) so a
/// title containing `*`, `_`, brackets etc. doesn't reformat the report.
fn md_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `embed=true` inlines the images as base64 data URIs (self-contained).
fn render_html(steps: &[Step], started: &str, dir: &Path, embed: bool) -> String {
    let t = crate::i18n::tr();
    let mut cards = String::new();
    for s in steps {
        // A manual step may carry no screenshot — then render text only.
        let img_tag = if s.image_file.is_empty() {
            String::new()
        } else {
            let src = if embed {
                match fs::read(dir.join(&s.image_file)) {
                    Ok(bytes) => format!("data:image/png;base64,{}", base64(&bytes)),
                    Err(_) => html_escape(&s.image_file), // fallback: file reference
                }
            } else {
                html_escape(&s.image_file)
            };
            let alt = html_escape(&t.report_step.replace("{n}", &s.index.to_string()));
            format!("\n    <img src=\"{src}\" alt=\"{alt}\" loading=\"lazy\">")
        };
        let meta_line = match &s.process {
            Some(p) if !p.is_empty() => {
                format!("{} · {}", html_escape(&s.time), html_escape(p))
            }
            _ => html_escape(&s.time),
        };
        cards.push_str(&format!(
            r#"  <section class="step">
    <div class="head"><span class="num">{n}</span>
      <div><p class="desc">{desc}</p><p class="time">{meta_line}</p></div>
    </div>{img_tag}
  </section>
"#,
            n = s.index,
            desc = html_escape(&s.describe()),
            meta_line = meta_line,
            img_tag = img_tag,
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
pub(crate) fn base64(data: &[u8]) -> String {
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

/// Minimal base64 decoder (standard alphabet, `=`/whitespace ignored). Returns
/// `None` on an invalid character. Counterpart to [`base64`], used to decode the
/// data-URI images the editor sends for manual steps.
pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lut = [255u8; 256];
    for (i, &c) in A.iter().enumerate() {
        lut[c as usize] = i as u8;
    }
    let (mut buf, mut bits) = (0u32, 0u32);
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for &b in s.as_bytes() {
        if matches!(b, b'=' | b'\n' | b'\r' | b' ' | b'\t') {
            continue;
        }
        let v = lut[b as usize];
        if v == 255 {
            return None;
        }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 test vectors — the hand-rolled encoder feeds every embedded
    /// image in the final report, so it gets the canonical vectors.
    #[test]
    fn base64_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_binary_roundtrip_length() {
        let data: Vec<u8> = (0..=255).collect();
        let enc = base64(&data);
        assert_eq!(enc.len(), data.len().div_ceil(3) * 4);
        assert_eq!(&enc[..8], "AAECAwQF");
    }

    #[test]
    fn base64_decode_roundtrips_and_rejects_junk() {
        for v in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foobar",
            &(0u8..=255).collect::<Vec<u8>>()[..],
        ] {
            assert_eq!(base64_decode(&base64(v)).unwrap(), v);
        }
        assert_eq!(base64_decode("Zm9v\nYmFy").unwrap(), b"foobar"); // whitespace ok
        assert!(base64_decode("not base64!!").is_none());
    }

    #[test]
    fn html_escape_covers_markup() {
        assert_eq!(
            html_escape(r#"<a href="x">&</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;"
        );
    }

    #[test]
    fn md_escape_neutralizes_syntax() {
        assert_eq!(md_escape("a*b_c"), r"a\*b\_c");
        assert_eq!(md_escape("[x](y)"), r"\[x\](y)");
        assert_eq!(md_escape("plain — text"), "plain — text");
    }
}
