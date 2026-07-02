//! Persistent configuration (`~/.config/stepshot/config.toml`).
//!
//! stepshot ships with sensible defaults and works with no config file at all;
//! the file only overrides. Parsing uses a tiny hand-rolled TOML **subset**
//! (sections, `key = value`, strings/bools/numbers/string-arrays, full-line
//! `#` comments) — dependency-free, matching the base64/i18n approach elsewhere.
//! Anything the parser doesn't understand is ignored, and a broken file falls
//! back to defaults with a warning rather than aborting startup.

use crate::annotate::MarkerStyle;
use crate::model::Button;
use std::collections::HashMap;
use std::path::PathBuf;

/// The resolved configuration (always valid — missing/broken files → defaults).
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// `[general] output_dir` — base folder for sessions. A CLI path argument
    /// still takes precedence over this.
    pub output_dir: Option<PathBuf>,
    /// `[marker]` — click-marker appearance.
    pub marker: MarkerStyle,
    /// `[export]` — which report formats to write.
    pub export: ExportConfig,
    /// `[capture]` — click handling (filtering, double-click merge).
    pub capture: CaptureConfig,
}

/// Click-capture behavior.
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Which mouse buttons create steps. Clicks of other buttons are ignored.
    pub buttons: Vec<Button>,
    /// Two clicks of the same button within this many milliseconds merge into a
    /// single “double click” step. `0` disables merging.
    pub double_click_ms: u64,
    /// A press that moves at least this many pixels before release is recorded
    /// as a drag-and-drop step rather than a click. `0` disables drag detection.
    pub drag_min_px: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            buttons: vec![Button::Left, Button::Right, Button::Middle],
            double_click_ms: 400,
            drag_min_px: 16,
        }
    }
}

impl CaptureConfig {
    /// Whether a click of `button` should be recorded.
    pub fn records(&self, button: Button) -> bool {
        self.buttons.contains(&button)
    }
}

/// Which report formats get written.
#[derive(Debug, Clone)]
pub struct ExportConfig {
    pub formats: Vec<Format>,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            formats: vec![Format::Html, Format::Md, Format::Pdf, Format::Docx],
        }
    }
}

impl ExportConfig {
    pub fn has(&self, f: Format) -> bool {
        self.formats.contains(&f)
    }
}

/// A report output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Html,
    Md,
    Pdf,
    Docx,
}

impl Format {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "html" => Some(Format::Html),
            "md" | "markdown" => Some(Format::Md),
            "pdf" => Some(Format::Pdf),
            "docx" | "word" => Some(Format::Docx),
            _ => None,
        }
    }
}

impl Config {
    /// Load from the standard path, or return defaults. Never fails: a missing
    /// file is normal, and a malformed one logs a warning and uses defaults.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Config::default();
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return Config::default(), // absent → defaults, silently
        };
        Config::from_toml_str(&text)
    }

    /// Parse config text (the file-less core, for tests).
    pub fn from_toml_str(text: &str) -> Self {
        let toml = Toml::parse(text);
        let mut cfg = Config::default();

        if let Some(dir) = toml.string("general", "output_dir") {
            cfg.output_dir = Some(PathBuf::from(dir));
        }

        let m = &mut cfg.marker;
        if let Some(c) = toml.color("marker", "fill") {
            m.fill = c;
        }
        if let Some(a) = toml.f32("marker", "fill_alpha") {
            m.fill_alpha = a.clamp(0.0, 1.0);
        }
        if let Some(c) = toml.color("marker", "rim") {
            m.rim = c;
        }
        if let Some(a) = toml.f32("marker", "rim_alpha") {
            m.rim_alpha = a.clamp(0.0, 1.0);
        }
        if let Some(r) = toml.f32("marker", "radius")
            && r > 0.0
        {
            m.radius = r;
        }

        if let Some(list) = toml.string_array("export", "formats") {
            let formats: Vec<Format> = list.iter().filter_map(|s| Format::parse(s)).collect();
            if !formats.is_empty() {
                cfg.export.formats = formats;
            }
        }

        if let Some(ms) = toml.u64("capture", "double_click_ms") {
            cfg.capture.double_click_ms = ms;
        }
        if let Some(px) = toml.u64("capture", "drag_min_px") {
            cfg.capture.drag_min_px = px as u32;
        }
        if let Some(list) = toml.string_array("capture", "buttons") {
            let buttons: Vec<Button> = list.iter().filter_map(|s| parse_button(s)).collect();
            if !buttons.is_empty() {
                cfg.capture.buttons = buttons;
            }
        }

        cfg
    }

    /// A fully-commented example config, for `--write-config` / docs.
    pub fn example_toml() -> &'static str {
        EXAMPLE_TOML
    }
}

/// `$XDG_CONFIG_HOME/stepshot/config.toml`, else `~/.config/…`, else (Windows)
/// `%APPDATA%\stepshot\config.toml`.
pub fn config_path() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(x).join("stepshot").join("config.toml"));
    }
    if let Some(h) = std::env::var_os("HOME").filter(|s| !s.is_empty()) {
        return Some(
            PathBuf::from(h)
                .join(".config")
                .join("stepshot")
                .join("config.toml"),
        );
    }
    if let Some(a) = std::env::var_os("APPDATA").filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(a).join("stepshot").join("config.toml"));
    }
    None
}

const EXAMPLE_TOML: &str = "\
# stepshot configuration. Every key is optional; delete any you don't need and
# the built-in default applies. Colors are \"#RRGGBB\"; alpha is 0.0-1.0.

[general]
# Base folder for sessions (a path given on the command line still wins).
# output_dir = \"~/Pictures/stepshot\"

[marker]
# The translucent click highlight drawn into each screenshot.
# fill = \"#FFE13C\"       # highlighter yellow
# fill_alpha = 0.35       # see-through so text under it stays readable
# rim = \"#FFA500\"        # amber outline for contrast on light backgrounds
# rim_alpha = 0.75
# radius = 20.0           # marker size in pixels

[export]
# Which report formats to write. Default: all of them.
# formats = [\"html\", \"md\", \"pdf\", \"docx\"]

[capture]
# Which mouse buttons create steps. Default: all three.
# buttons = [\"left\", \"right\", \"middle\"]
# Two clicks of the same button within this many milliseconds merge into one
# \"double click\" step. Set to 0 to disable merging. Default: 400.
# double_click_ms = 400
# A press that moves at least this many pixels before release becomes a
# drag-and-drop step instead of a click. Set to 0 to disable. Default: 16.
# drag_min_px = 16
";

// ─────────────────────────── minimal TOML subset ───────────────────────────

/// Parsed sections → (key → raw right-hand-side string).
struct Toml {
    sections: HashMap<String, HashMap<String, String>>,
}

impl Toml {
    fn parse(text: &str) -> Self {
        let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut current = String::new(); // top-level keys live under ""
        sections.entry(current.clone()).or_default();

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                current = name.trim().to_string();
                sections.entry(current.clone()).or_default();
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                sections
                    .entry(current.clone())
                    .or_default()
                    .insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        Toml { sections }
    }

    fn raw(&self, section: &str, key: &str) -> Option<&str> {
        self.sections.get(section)?.get(key).map(|s| s.as_str())
    }

    fn string(&self, section: &str, key: &str) -> Option<String> {
        self.raw(section, key).map(unquote)
    }

    fn f32(&self, section: &str, key: &str) -> Option<f32> {
        self.raw(section, key)?.trim().parse().ok()
    }

    fn u64(&self, section: &str, key: &str) -> Option<u64> {
        self.raw(section, key)?.trim().parse().ok()
    }

    fn color(&self, section: &str, key: &str) -> Option<[u8; 3]> {
        parse_hex_color(&unquote(self.raw(section, key)?))
    }

    fn string_array(&self, section: &str, key: &str) -> Option<Vec<String>> {
        let raw = self.raw(section, key)?.trim();
        let inner = raw.strip_prefix('[')?.strip_suffix(']')?;
        Some(
            inner
                .split(',')
                .map(|s| unquote(s.trim()))
                .filter(|s| !s.is_empty())
                .collect(),
        )
    }
}

/// Strip surrounding double or single quotes if present.
fn unquote(s: &str) -> String {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// `left` / `right` / `middle` → [`Button`].
fn parse_button(s: &str) -> Option<Button> {
    match s.trim().to_lowercase().as_str() {
        "left" => Some(Button::Left),
        "right" => Some(Button::Right),
        "middle" => Some(Button::Middle),
        _ => None,
    }
}

/// `#RRGGBB` or `RRGGBB` → `[r, g, b]`.
fn parse_hex_color(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some([r, g, b])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let c = Config::from_toml_str("");
        assert!(c.output_dir.is_none());
        assert_eq!(c.marker.fill, [255, 225, 60]);
        assert_eq!(c.export.formats.len(), 4);
        assert!(c.export.has(Format::Pdf));
    }

    #[test]
    fn parses_all_sections() {
        let c = Config::from_toml_str(
            r##"
            # a comment
            [general]
            output_dir = "/tmp/shots"

            [marker]
            fill = "#102030"
            fill_alpha = 0.5
            radius = 12

            [export]
            formats = ["html", "pdf"]
            "##,
        );
        assert_eq!(c.output_dir, Some(PathBuf::from("/tmp/shots")));
        assert_eq!(c.marker.fill, [16, 32, 48]);
        assert!((c.marker.fill_alpha - 0.5).abs() < 1e-6);
        assert!((c.marker.radius - 12.0).abs() < 1e-6);
        assert_eq!(c.export.formats, vec![Format::Html, Format::Pdf]);
    }

    #[test]
    fn bad_values_fall_back_to_defaults() {
        let c = Config::from_toml_str(
            r#"
            [marker]
            fill = "not-a-color"
            fill_alpha = 9.0
            radius = -5

            [export]
            formats = ["nope", "also-nope"]
            "#,
        );
        assert_eq!(c.marker.fill, [255, 225, 60]); // unchanged
        assert!((c.marker.fill_alpha - 1.0).abs() < 1e-6); // clamped
        assert!((c.marker.radius - 20.0).abs() < 1e-6); // rejected
        assert_eq!(c.export.formats.len(), 4); // empty → default
    }

    #[test]
    fn hex_and_unquote() {
        assert_eq!(parse_hex_color("#FFA500"), Some([255, 165, 0]));
        assert_eq!(parse_hex_color("ffa500"), Some([255, 165, 0]));
        assert_eq!(parse_hex_color("#abc"), None);
        assert_eq!(unquote("\"hi\""), "hi");
        assert_eq!(unquote("'hi'"), "hi");
        assert_eq!(unquote("bare"), "bare");
    }

    #[test]
    fn capture_defaults_and_parsing() {
        let d = Config::from_toml_str("");
        assert_eq!(d.capture.double_click_ms, 400);
        assert_eq!(d.capture.drag_min_px, 16);
        assert_eq!(d.capture.buttons.len(), 3);
        assert!(d.capture.records(Button::Middle));

        let c = Config::from_toml_str(
            r#"
            [capture]
            buttons = ["left", "nope", "right"]
            double_click_ms = 250
            drag_min_px = 32
            "#,
        );
        assert_eq!(c.capture.double_click_ms, 250);
        assert_eq!(c.capture.drag_min_px, 32);
        assert_eq!(c.capture.buttons, vec![Button::Left, Button::Right]);
        assert!(c.capture.records(Button::Left));
        assert!(!c.capture.records(Button::Middle)); // filtered out

        // An all-invalid button list keeps the default (all three).
        let f = Config::from_toml_str("[capture]\nbuttons = [\"nope\"]\n");
        assert_eq!(f.capture.buttons.len(), 3);
    }

    #[test]
    fn format_aliases() {
        assert_eq!(Format::parse("markdown"), Some(Format::Md));
        assert_eq!(Format::parse("WORD"), Some(Format::Docx));
        assert_eq!(Format::parse("xls"), None);
    }
}
