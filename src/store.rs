//! `session.json` — the per-session source of truth on disk.
//!
//! Serializes a [`Session`] (written incrementally after each step, crash-safe,
//! and again on finalize) and reads it back for `stepshot apply` / `stepshot
//! edit`, which rebuild every export from it.

use crate::json::Json;
use crate::model::{Button, KeyKind, Step};
use crate::session::Session;
use anyhow::{Context, Result};
use std::path::Path;

/// Serialize the session: everything `stepshot apply` needs to rebuild every
/// export, and everything the in-report editor needs (per-step metadata + the
/// clicked element's box in image pixels).
pub fn session_json(s: &Session) -> String {
    let steps: Vec<Json> = s.steps.iter().map(step_json).collect();
    Json::obj(vec![
        ("schema", 1u32.into()),
        ("app", "stepshot".into()),
        ("version", env!("CARGO_PKG_VERSION").into()),
        ("language", crate::i18n::tr().html_lang.into()),
        ("started", s.started.as_str().into()),
        ("steps", Json::Arr(steps)),
    ])
    .to_pretty()
}

fn step_json(step: &Step) -> Json {
    let button = match step.button {
        Button::Left => "left",
        Button::Right => "right",
        Button::Middle => "middle",
    };
    let element_box = match step.element_box {
        Some([x, y, w, h]) => Json::Arr(vec![x.into(), y.into(), w.into(), h.into()]),
        None => Json::Null,
    };
    let key: Json = match step.key {
        Some(KeyKind::Text) => "text".into(),
        Some(KeyKind::Password) => "password".into(),
        Some(KeyKind::Press) => "press".into(),
        None => Json::Null,
    };
    Json::obj(vec![
        ("index", step.index.into()),
        ("button", button.into()),
        ("key", key),
        ("keys", step.keys.clone().into()),
        ("double", step.double.into()),
        ("drag", step.drag.into()),
        ("time", step.time.as_str().into()),
        ("image", step.image_file.as_str().into()),
        ("is_screen", step.is_screen.into()),
        ("window_title", step.window_title.clone().into()),
        ("process", step.process.clone().into()),
        ("element", step.element.clone().into()),
        ("element_box", element_box),
        // The auto-generated text stays here so an override can be reverted…
        ("description", step.auto_describe().into()),
        // …while the editor's replacement (if any) rides alongside it.
        (
            "description_override",
            step.description_override.clone().into(),
        ),
    ])
}

/// Write `session.json` (no-op for 0 steps). Called incrementally after each
/// step (crash-safe) and again on finalize.
pub fn write_session_json(s: &Session) {
    if s.steps.is_empty() {
        return;
    }
    if let Err(e) = std::fs::write(s.dir.join("session.json"), session_json(s)) {
        eprintln!("[stepshot] could not write session.json: {e}");
    }
}

/// A session loaded back from its `session.json`, plus the language it was
/// recorded in (so `apply` can render in the captured locale).
pub struct LoadedSession {
    pub session: Session,
    pub language: String,
}

/// Read `session.json` from a session folder back into a [`Session`].
pub fn load_session(dir: &Path) -> Result<LoadedSession> {
    let path = dir.join("session.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let j = Json::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let language = j
        .get("language")
        .and_then(Json::as_str)
        .unwrap_or("en")
        .to_string();
    let started = j
        .get("started")
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    let steps_json = j
        .get("steps")
        .and_then(Json::as_array)
        .context("session.json has no \"steps\" array")?;
    let steps = steps_json
        .iter()
        .map(step_from_json)
        .collect::<Result<_>>()?;
    Ok(LoadedSession {
        session: Session {
            dir: dir.to_path_buf(),
            started,
            steps,
        },
        language,
    })
}

fn step_from_json(j: &Json) -> Result<Step> {
    let index = j
        .get("index")
        .and_then(Json::as_i64)
        .context("step missing \"index\"")? as usize;
    let button = match j.get("button").and_then(Json::as_str) {
        Some("right") => Button::Right,
        Some("middle") => Button::Middle,
        _ => Button::Left,
    };
    let opt_str = |k: &str| j.get(k).and_then(Json::as_str).map(str::to_string);
    let flag = |k: &str| j.get(k).and_then(Json::as_bool).unwrap_or(false);
    Ok(Step {
        index,
        button,
        time: opt_str("time").unwrap_or_default(),
        image_file: opt_str("image").unwrap_or_default(),
        window_title: opt_str("window_title"),
        process: opt_str("process"),
        element: opt_str("element"),
        element_box: j.get("element_box").and_then(Json::as_u32x4),
        description_override: opt_str("description_override"),
        is_screen: flag("is_screen"),
        double: flag("double"),
        drag: flag("drag"),
        key: match j.get("key").and_then(Json::as_str) {
            Some("text") => Some(KeyKind::Text),
            Some("password") => Some(KeyKind::Password),
            Some("press") => Some(KeyKind::Press),
            _ => None,
        },
        keys: opt_str("keys"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn step(is_screen: bool, window_title: Option<&str>) -> Step {
        Step {
            index: 1,
            time: "12:00:00".into(),
            image_file: "step-001.png".into(),
            window_title: window_title.map(String::from),
            is_screen,
            ..Step::default()
        }
    }

    #[test]
    fn session_json_captures_key_fields() {
        let mut s0 = step(false, Some("Editor"));
        s0.process = Some("kitty".into());
        s0.element_box = Some([1, 2, 3, 4]);
        let sess = Session {
            dir: PathBuf::from("/tmp/x"),
            started: "2026-07-02 13:00:00".into(),
            steps: vec![s0],
        };
        let j = Json::parse(&session_json(&sess)).unwrap();
        assert_eq!(j.get("schema").and_then(Json::as_i64), Some(1));
        let steps = j.get("steps").and_then(Json::as_array).unwrap();
        assert_eq!(steps.len(), 1);
        let st = &steps[0];
        assert_eq!(st.get("button").and_then(Json::as_str), Some("left"));
        assert_eq!(st.get("process").and_then(Json::as_str), Some("kitty"));
        assert_eq!(
            st.get("window_title").and_then(Json::as_str),
            Some("Editor")
        );
        let bx: Vec<i64> = st
            .get("element_box")
            .and_then(Json::as_array)
            .unwrap()
            .iter()
            .filter_map(Json::as_i64)
            .collect();
        assert_eq!(bx, vec![1, 2, 3, 4]);
        assert_eq!(
            st.get("description").and_then(Json::as_str),
            Some("Left click in window “Editor”")
        );
    }

    #[test]
    fn session_json_load_roundtrips_steps() {
        let mut a = step(false, Some("Editor"));
        a.process = Some("kitty".into());
        a.element_box = Some([5, 6, 7, 8]);
        a.description_override = Some("custom".into());
        let mut b = step(true, None);
        b.button = Button::Right;
        b.double = true;
        b.index = 2;
        let sess = Session {
            dir: PathBuf::from("/tmp/x"),
            started: "2026-07-02 13:00:00".into(),
            steps: vec![a, b],
        };
        let text = session_json(&sess);
        // Reload from an in-memory copy (load_session reads a file, so mirror it).
        let j = Json::parse(&text).unwrap();
        let steps = j.get("steps").and_then(Json::as_array).unwrap();
        let s0 = step_from_json(&steps[0]).unwrap();
        let s1 = step_from_json(&steps[1]).unwrap();
        assert_eq!(s0.process.as_deref(), Some("kitty"));
        assert_eq!(s0.element_box, Some([5, 6, 7, 8]));
        assert_eq!(s0.description_override.as_deref(), Some("custom"));
        assert_eq!(s0.describe(), "custom"); // override wins on reload
        assert_eq!(s1.button, Button::Right);
        assert!(s1.double && s1.is_screen);
        assert_eq!(s1.index, 2);
    }
}
