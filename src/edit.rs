//! `stepshot edit <session-dir>` — the native in-browser editor.
//!
//! A browser page can't write to disk, so instead of downloading an
//! `edits.json` and running `stepshot apply` by hand, this serves the editor
//! from a tiny loopback HTTP server: "Apply" POSTs the edits straight back and
//! [`crate::apply`] rewrites the session in place. Zero dependencies — a
//! hand-rolled HTTP/1.1 handler over `std::net`, matching the rest of the code.
//!
//! Bound to `127.0.0.1` only, and every request must carry the one-time `token`
//! baked into the URL, so other local processes can't drive it.

use crate::apply;
use crate::b64::base64;
use crate::json::Json;
use crate::report::html_escape;
use crate::store;
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

/// Start the editor server for `session_dir`, open a browser at it, and serve
/// until the user clicks "Done" (or the process is interrupted).
pub fn run(session_dir: &Path) -> Result<()> {
    // Fail early with a clear message if there's nothing to edit.
    let loaded = store::load_session(session_dir)
        .with_context(|| format!("no readable session.json in {}", session_dir.display()))?;
    crate::i18n::init_lang(&loaded.language);
    drop(loaded);

    let listener =
        TcpListener::bind("127.0.0.1:0").context("could not bind a local editor port")?;
    let port = listener.local_addr()?.port();
    let token = make_token();
    let url = format!("http://127.0.0.1:{port}/?token={token}");

    println!("stepshot editor running at {url}");
    println!("(leave this running while you edit; press Ctrl+C or click “Done” to stop)");
    // STEPSHOT_NO_OPEN skips launching a browser (headless/testing).
    if std::env::var_os("STEPSHOT_NO_OPEN").is_none() {
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }

    for conn in listener.incoming() {
        let stream = match conn {
            Ok(s) => s,
            Err(_) => continue,
        };
        match handle(stream, session_dir, &token) {
            Ok(true) => break, // "Done"
            Ok(false) => {}
            Err(e) => eprintln!("[stepshot] editor request failed: {e:#}"),
        }
    }
    println!("stepshot editor stopped.");
    Ok(())
}

/// Handle one connection. Returns `Ok(true)` when the user asked to stop.
fn handle(mut stream: TcpStream, dir: &Path, token: &str) -> Result<bool> {
    let req = Request::read(&mut stream)?;

    if req.query_token().as_deref() != Some(token) {
        respond(&mut stream, 403, "text/plain; charset=utf-8", b"forbidden");
        return Ok(false);
    }

    match (req.method.as_str(), req.route()) {
        ("GET", "/") => {
            let html = build_page(dir)?;
            respond(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                html.as_bytes(),
            );
            Ok(false)
        }
        ("POST", "/apply") => {
            let body = match apply::apply_text(dir, &req.body) {
                Ok(n) => Json::obj(vec![("ok", true.into()), ("steps", n.into())]),
                Err(e) => Json::obj(vec![
                    ("ok", false.into()),
                    ("error", format!("{e:#}").into()),
                ]),
            }
            .to_compact();
            respond(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                body.as_bytes(),
            );
            Ok(false)
        }
        ("GET" | "POST", "/done") => {
            respond(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                b"<p style=\"font:1rem system-ui;padding:2rem\">stepshot editor closed. You can close this tab.</p>",
            );
            Ok(true)
        }
        _ => {
            respond(&mut stream, 404, "text/plain; charset=utf-8", b"not found");
            Ok(false)
        }
    }
}

// ─────────────────────────── minimal HTTP ───────────────────────────

/// A parsed request: just what the editor protocol needs.
struct Request {
    method: String,
    target: String, // path plus optional `?query`
    body: String,
}

impl Request {
    fn read(stream: &mut TcpStream) -> Result<Request> {
        let mut reader = BufReader::new(stream.try_clone().context("could not clone stream")?);

        let mut line = String::new();
        reader.read_line(&mut line)?;
        let mut it = line.split_whitespace();
        let method = it.next().unwrap_or_default().to_string();
        let target = it.next().unwrap_or_default().to_string();

        let mut content_length = 0usize;
        loop {
            let mut h = String::new();
            let n = reader.read_line(&mut h)?;
            if n == 0 || h == "\r\n" || h == "\n" {
                break; // end of headers
            }
            if let Some((k, v)) = h.split_once(':')
                && k.trim().eq_ignore_ascii_case("content-length")
            {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
        // Cap the body so a bogus Content-Length can't exhaust memory.
        content_length = content_length.min(16 * 1024 * 1024);
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body)?;

        Ok(Request {
            method,
            target,
            body: String::from_utf8_lossy(&body).into_owned(),
        })
    }

    fn route(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    fn query_token(&self) -> Option<String> {
        let q = self.target.split_once('?')?.1;
        q.split('&')
            .find_map(|kv| kv.strip_prefix("token=").map(str::to_string))
    }
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "OK",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// A per-run token so only our page (which carries it in the URL) can drive the
/// server. Not cryptographic — just a nonce other local apps won't guess.
fn make_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}{:x}", std::process::id(), nanos)
}

// ─────────────────────────── editor page ───────────────────────────

/// Render the editor HTML from the current on-disk session (regenerated on every
/// GET, so it always reflects the latest applied state).
fn build_page(dir: &Path) -> Result<String> {
    let loaded = store::load_session(dir)?;
    let sess = loaded.session;
    let t = crate::i18n::tr();

    let mut cards = String::new();
    for s in &sess.steps {
        let bytes = std::fs::read(dir.join(&s.image_file)).unwrap_or_default();
        let img_src = format!("data:image/png;base64,{}", base64(&bytes));
        let elem_btn = match s.element_box {
            Some([x, y, w, h]) => format!(
                "<button type=\"button\" class=\"elembtn\" data-box=\"[{x},{y},{w},{h}]\">{}</button>",
                html_escape(t.edit_redact_element)
            ),
            None => String::new(),
        };
        cards.push_str(&format!(
            r#"<section class="step" data-ref="{ref}" data-auto="{auto}">
  <div class="bar">
    <span class="num">{ref}</span>
    <button type="button" class="mv up" title="▲">▲</button>
    <button type="button" class="mv down" title="▼">▼</button>
    <button type="button" class="del">{del}</button>
    {elem_btn}
  </div>
  <input class="desc" value="{desc}">
  <div class="shot"><img src="{img_src}" alt="step {ref}"><div class="overlay"></div></div>
</section>
"#,
            ref = s.index,
            auto = html_escape(&s.auto_describe()),
            del = html_escape(t.edit_delete),
            elem_btn = elem_btn,
            desc = html_escape(&s.describe()),
            img_src = img_src,
        ));
    }

    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>stepshot — {heading}</title>
<style>{css}</style>
</head>
<body>
<header>
  <h1>{heading}</h1>
  <p class="banner">{banner}</p>
  <p class="hint">{hint}</p>
</header>
<main id="steps">
{cards}</main>
<footer>
  <button id="apply" data-applied="{applied}" data-error="{error}">{apply}</button>
  <button id="addstep" data-text="{manual_text}" data-del="{del}">＋ {add_step}</button>
  <button id="done">{done}</button>
  <span id="status"></span>
</footer>
<script>{js}</script>
</body>
</html>
"#,
        lang = t.html_lang,
        heading = html_escape(t.report_heading),
        banner = html_escape(t.edit_banner),
        hint = html_escape(t.edit_hint),
        apply = html_escape(t.edit_apply),
        done = html_escape(t.edit_done),
        applied = html_escape(t.edit_applied),
        error = html_escape(t.edit_error),
        add_step = html_escape(t.edit_add_step),
        manual_text = html_escape(t.edit_manual_text),
        del = html_escape(t.edit_delete),
        css = EDITOR_CSS,
        cards = cards,
        js = EDITOR_JS,
    ))
}

/// Editor stylesheet — its own file for syntax highlighting and diffs.
const EDITOR_CSS: &str = include_str!("editor.css");

/// Editor client logic (redaction boxes, reorder, manual steps, apply POST).
const EDITOR_JS: &str = include_str!("editor.js");
