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
use crate::report::{base64, html_escape};
use crate::session;
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

/// Start the editor server for `session_dir`, open a browser at it, and serve
/// until the user clicks "Done" (or the process is interrupted).
pub fn run(session_dir: &Path) -> Result<()> {
    // Fail early with a clear message if there's nothing to edit.
    let loaded = session::load_session(session_dir)
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
                Ok(n) => format!("{{\"ok\":true,\"steps\":{n}}}"),
                Err(e) => format!(
                    "{{\"ok\":false,\"error\":{}}}",
                    json_string(&format!("{e:#}"))
                ),
            };
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

/// Minimal JSON string escaping for the error message we echo back.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ─────────────────────────── editor page ───────────────────────────

/// Render the editor HTML from the current on-disk session (regenerated on every
/// GET, so it always reflects the latest applied state).
fn build_page(dir: &Path) -> Result<String> {
    let loaded = session::load_session(dir)?;
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

const EDITOR_CSS: &str = r#"
  :root { color-scheme: light dark; }
  body { font-family: system-ui, sans-serif; max-width: 980px; margin: 0 auto 6rem; padding: 0 1rem; line-height: 1.5; }
  header { position: sticky; top: 0; background: Canvas; border-bottom: 2px solid #8884; padding: .75rem 0; z-index: 5; }
  h1 { margin: 0; font-size: 1.4rem; }
  .banner { margin: .3rem 0 0; font-weight: 600; }
  .hint { margin: .1rem 0 0; color: #8888; font-size: .9rem; }
  .step { margin: 1.5rem 0; padding-top: 1rem; border-top: 1px solid #8883; }
  .step.deleted { opacity: .35; }
  .step.deleted .shot { pointer-events: none; }
  .bar { display: flex; align-items: center; gap: .5rem; margin-bottom: .5rem; flex-wrap: wrap; }
  .num { flex: 0 0 auto; width: 1.8rem; height: 1.8rem; border-radius: 50%; background: #3b82f6; color: #fff; display: grid; place-items: center; font-weight: 700; }
  button { font: inherit; padding: .3rem .7rem; border: 1px solid #8886; border-radius: 6px; background: #8881; cursor: pointer; }
  button:hover { background: #8883; }
  .mv { padding: .2rem .5rem; }
  .desc { width: 100%; font: inherit; padding: .4rem .5rem; border: 1px solid #8886; border-radius: 6px; box-sizing: border-box; margin-bottom: .5rem; background: Field; color: FieldText; }
  .shot { position: relative; display: inline-block; max-width: 100%; }
  .shot img { max-width: 100%; height: auto; display: block; border: 1px solid #8884; border-radius: 8px; cursor: crosshair; }
  .overlay { position: absolute; inset: 0; }
  .rbox { position: absolute; background: rgba(20,20,20,.55); border: 1px solid #fff8; cursor: pointer; }
  .rbox.temp { background: rgba(20,20,20,.3); pointer-events: none; }
  footer { position: fixed; bottom: 0; left: 0; right: 0; background: Canvas; border-top: 2px solid #8884; padding: .6rem 1rem; display: flex; gap: .8rem; align-items: center; }
  #apply { background: #3b82f6; color: #fff; border-color: #3b82f6; font-weight: 700; }
  #status { color: #8888; }
"#;

const EDITOR_JS: &str = r#"
(function () {
  const params = new URLSearchParams(location.search);
  const token = params.get('token') || '';
  const steps = document.getElementById('steps');

  document.querySelectorAll('.step').forEach(setupStep);

  function setupStep(card) {
    const shot = card.querySelector('.shot');
    if (!shot) return; // manual (text-only) step — nothing to redact
    const img = shot.querySelector('img');
    const overlay = shot.querySelector('.overlay');
    let boxes = [];            // natural image-pixel [x,y,w,h]
    let start = null, tempEl = null;

    const scale = () => img.naturalWidth / (img.clientWidth || img.naturalWidth);

    function persist() { card.dataset.redact = JSON.stringify(boxes); }
    function render() {
      overlay.querySelectorAll('.rbox:not(.temp)').forEach(e => e.remove());
      const s = scale();
      boxes.forEach((b, i) => {
        const d = document.createElement('div');
        d.className = 'rbox';
        d.style.left = (b[0] / s) + 'px'; d.style.top = (b[1] / s) + 'px';
        d.style.width = (b[2] / s) + 'px'; d.style.height = (b[3] / s) + 'px';
        d.title = '✕';
        d.addEventListener('click', ev => { ev.stopPropagation(); boxes.splice(i, 1); persist(); render(); });
        overlay.appendChild(d);
      });
    }
    function addBox(a, b) {
      const s = scale();
      const x = Math.min(a.x, b.x), y = Math.min(a.y, b.y);
      const w = Math.abs(a.x - b.x), h = Math.abs(a.y - b.y);
      if (w < 4 || h < 4) return;
      boxes.push([Math.round(x * s), Math.round(y * s), Math.round(w * s), Math.round(h * s)]);
      persist(); render();
    }

    shot.addEventListener('mousedown', e => {
      if (e.button !== 0 || e.target.classList.contains('rbox')) return;
      const r = img.getBoundingClientRect();
      start = { x: e.clientX - r.left, y: e.clientY - r.top };
      tempEl = document.createElement('div');
      tempEl.className = 'rbox temp';
      overlay.appendChild(tempEl);
      e.preventDefault();
    });
    window.addEventListener('mousemove', e => {
      if (!start) return;
      const r = img.getBoundingClientRect();
      const cx = e.clientX - r.left, cy = e.clientY - r.top;
      tempEl.style.left = Math.min(start.x, cx) + 'px';
      tempEl.style.top = Math.min(start.y, cy) + 'px';
      tempEl.style.width = Math.abs(cx - start.x) + 'px';
      tempEl.style.height = Math.abs(cy - start.y) + 'px';
    });
    window.addEventListener('mouseup', e => {
      if (!start) return;
      const r = img.getBoundingClientRect();
      addBox(start, { x: e.clientX - r.left, y: e.clientY - r.top });
      start = null;
      if (tempEl) { tempEl.remove(); tempEl = null; }
    });

    const elemBtn = card.querySelector('.elembtn');
    if (elemBtn) elemBtn.addEventListener('click', () => {
      const b = JSON.parse(elemBtn.dataset.box);
      boxes.push(b); persist(); render();
    });

    window.addEventListener('resize', render);
    img.addEventListener('load', render);
  }

  function renumber() {
    let n = 0;
    steps.querySelectorAll('.step').forEach(c => {
      c.querySelector('.num').textContent = c.classList.contains('deleted') ? '—' : (++n);
    });
  }

  steps.addEventListener('click', e => {
    const card = e.target.closest('.step');
    if (!card) return;
    if (e.target.classList.contains('up') && card.previousElementSibling)
      card.parentNode.insertBefore(card, card.previousElementSibling);
    else if (e.target.classList.contains('down') && card.nextElementSibling)
      card.parentNode.insertBefore(card.nextElementSibling, card);
    else if (e.target.classList.contains('del'))
      card.classList.toggle('deleted');
    renumber();
  });

  // Insert a manual (text, optional image) step at the end.
  const addBtn = document.getElementById('addstep');
  addBtn.addEventListener('click', () => {
    const card = document.createElement('section');
    card.className = 'step manual';
    card.dataset.manual = '1';
    card.innerHTML =
      '<div class="bar"><span class="num">+</span>' +
      '<button type="button" class="mv up" title="▲">▲</button>' +
      '<button type="button" class="mv down" title="▼">▼</button>' +
      '<button type="button" class="del">' + addBtn.dataset.del + '</button></div>' +
      '<input class="desc" placeholder="' + addBtn.dataset.text + '">' +
      '<input class="mfile" type="file" accept="image/*">';
    steps.appendChild(card);
    renumber();
    card.querySelector('.desc').focus();
  });

  function readAsDataURL(file) {
    return new Promise((resolve, reject) => {
      const r = new FileReader();
      r.onload = () => resolve(r.result);
      r.onerror = reject;
      r.readAsDataURL(file);
    });
  }

  async function gather() {
    const entries = [];
    for (const card of steps.querySelectorAll('.step')) {
      if (card.classList.contains('deleted')) continue;
      if (card.dataset.manual) {
        const text = card.querySelector('.desc').value.trim();
        if (!text) continue; // skip empty manual steps
        const entry = { text };
        const file = card.querySelector('.mfile').files[0];
        if (file) entry.image = await readAsDataURL(file);
        entries.push(entry);
      } else {
        const ref = +card.dataset.ref;
        const auto = card.dataset.auto;
        const desc = card.querySelector('.desc').value;
        const redact = JSON.parse(card.dataset.redact || '[]');
        const entry = { ref, description: desc === auto ? null : desc };
        if (redact.length) entry.redact = redact;
        entries.push(entry);
      }
    }
    return entries;
  }

  const applyBtn = document.getElementById('apply');
  const status = document.getElementById('status');
  applyBtn.addEventListener('click', async () => {
    const entries = await gather();
    status.textContent = '…';
    try {
      const res = await fetch('/apply?token=' + encodeURIComponent(token), {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ steps: entries })
      });
      const j = await res.json();
      if (j.ok) {
        status.textContent = applyBtn.dataset.applied.replace('{n}', j.steps);
        setTimeout(() => location.reload(), 700);
      } else {
        status.textContent = applyBtn.dataset.error + ': ' + (j.error || '');
      }
    } catch (err) {
      status.textContent = applyBtn.dataset.error + ': ' + err;
    }
  });

  document.getElementById('done').addEventListener('click', async () => {
    try { await fetch('/done?token=' + encodeURIComponent(token)); } catch (e) {}
    document.body.innerHTML = '<p style="padding:2rem">stepshot editor closed. You can close this tab.</p>';
  });
})();
"#;
