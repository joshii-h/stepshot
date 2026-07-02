//! Minimal, dependency-free JSON (the RFC 8259 subset we actually use).
//!
//! stepshot writes `session.json` (the source of truth for a recording) and
//! later reads back both it and the `edits.json` produced by the in-report
//! editor. We control both formats, so this is a compact reader/writer for
//! exactly that — objects, arrays, strings (with the standard escapes including
//! `\uXXXX` surrogate pairs), numbers, booleans and null. Same spirit as the
//! hand-rolled base64 / TOML-subset code elsewhere: no runtime dependency.
//!
//! The reader (`Json::parse` + the accessors) is exercised by the unit tests
//! here and gets its first non-test caller in `stepshot apply` (the editing
//! milestone); it deliberately lands together with the writer, so allow it to
//! sit ahead of that caller.
#![allow(dead_code)]

use std::fmt::Write as _;

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    // ── construction ────────────────────────────────────────────────────

    /// Build an object from `&str` keys (order-preserving).
    pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    // ── accessors (used by `apply` when reading edits back) ─────────────

    /// Look up a key in an object (`None` for non-objects / missing keys).
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(p) => p.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Json::Str(s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        if let Json::Num(n) = self {
            Some(*n)
        } else {
            None
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        self.as_f64().map(|n| n as i64)
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Json::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }
    pub fn as_array(&self) -> Option<&[Json]> {
        if let Json::Arr(a) = self {
            Some(a)
        } else {
            None
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    // ── writer ──────────────────────────────────────────────────────────

    /// Compact serialization (no whitespace).
    pub fn to_compact(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, None, 0);
        out
    }

    /// Indented serialization (2 spaces) — a readable `session.json`.
    pub fn to_pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, Some(2), 0);
        out
    }

    fn write(&self, out: &mut String, indent: Option<usize>, depth: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if !n.is_finite() {
                    out.push_str("null"); // JSON has no NaN/Infinity
                } else if n.fract() == 0.0 && n.abs() < 9e15 {
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    let _ = write!(out, "{n}");
                }
            }
            Json::Str(s) => write_escaped(out, s),
            Json::Arr(a) => {
                if a.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline_indent(out, indent, depth + 1);
                    v.write(out, indent, depth + 1);
                }
                newline_indent(out, indent, depth);
                out.push(']');
            }
            Json::Obj(p) => {
                if p.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push('{');
                for (i, (k, v)) in p.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline_indent(out, indent, depth + 1);
                    write_escaped(out, k);
                    out.push(':');
                    if indent.is_some() {
                        out.push(' ');
                    }
                    v.write(out, indent, depth + 1);
                }
                newline_indent(out, indent, depth);
                out.push('}');
            }
        }
    }

    // ── reader ──────────────────────────────────────────────────────────

    /// Parse a complete JSON document. Trailing non-whitespace is an error.
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser {
            b: text.as_bytes(),
            i: 0,
        };
        p.ws();
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("trailing data at byte {}", p.i));
        }
        Ok(v)
    }
}

fn newline_indent(out: &mut String, indent: Option<usize>, depth: usize) {
    if let Some(n) = indent {
        out.push('\n');
        for _ in 0..n * depth {
            out.push(' ');
        }
    }
}

fn write_escaped(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// Ergonomic building: `1u32.into()`, `"x".into()`, `Some(s).into()`, …
impl From<&str> for Json {
    fn from(s: &str) -> Json {
        Json::Str(s.to_string())
    }
}
impl From<String> for Json {
    fn from(s: String) -> Json {
        Json::Str(s)
    }
}
impl From<bool> for Json {
    fn from(b: bool) -> Json {
        Json::Bool(b)
    }
}
impl From<u32> for Json {
    fn from(n: u32) -> Json {
        Json::Num(n as f64)
    }
}
impl From<usize> for Json {
    fn from(n: usize) -> Json {
        Json::Num(n as f64)
    }
}
impl From<i64> for Json {
    fn from(n: i64) -> Json {
        Json::Num(n as f64)
    }
}
impl From<f64> for Json {
    fn from(n: f64) -> Json {
        Json::Num(n)
    }
}
impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(o: Option<T>) -> Json {
        match o {
            Some(v) => v.into(),
            None => Json::Null,
        }
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while let Some(c) = self.peek() {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(format!("unexpected byte '{}' at {}", c as char, self.i)),
            None => Err("unexpected end of input".into()),
        }
    }

    fn lit(&mut self, word: &str, val: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(val)
        } else {
            Err(format!("invalid literal at byte {}", self.i))
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.i += 1; // consume '{'
        let mut pairs = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Json::Obj(pairs));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(format!("expected key string at byte {}", self.i));
            }
            let key = self.string()?;
            self.ws();
            if self.peek() != Some(b':') {
                return Err(format!("expected ':' at byte {}", self.i));
            }
            self.i += 1;
            self.ws();
            let val = self.value()?;
            pairs.push((key, val));
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.i)),
            }
        }
        Ok(Json::Obj(pairs))
    }

    fn array(&mut self) -> Result<Json, String> {
        self.i += 1; // consume '['
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.i)),
            }
        }
        Ok(Json::Arr(items))
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // consume opening '"'
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i).ok_or("unterminated string")?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i).ok_or("truncated escape")?;
                    self.i += 1;
                    match e {
                        b'"' => buf.push(b'"'),
                        b'\\' => buf.push(b'\\'),
                        b'/' => buf.push(b'/'),
                        b'n' => buf.push(b'\n'),
                        b'r' => buf.push(b'\r'),
                        b't' => buf.push(b'\t'),
                        b'b' => buf.push(0x08),
                        b'f' => buf.push(0x0c),
                        b'u' => {
                            let ch = self.unicode_escape()?;
                            let mut tmp = [0u8; 4];
                            buf.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                        }
                        _ => return Err(format!("bad escape \\{}", e as char)),
                    }
                }
                // Any other byte belongs to a UTF-8 sequence from a valid &str,
                // so passing it through verbatim preserves the encoding.
                _ => buf.push(c),
            }
        }
        String::from_utf8(buf).map_err(|_| "invalid UTF-8 in string".to_string())
    }

    /// Read four hex digits into a code unit.
    fn hex4(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = *self.b.get(self.i).ok_or("truncated \\u escape")?;
            self.i += 1;
            let n = match d {
                b'0'..=b'9' => (d - b'0') as u32,
                b'a'..=b'f' => (d - b'a' + 10) as u32,
                b'A'..=b'F' => (d - b'A' + 10) as u32,
                _ => return Err("bad hex digit in \\u escape".into()),
            };
            v = v * 16 + n;
        }
        Ok(v)
    }

    /// A `\uXXXX` escape (the `\u` already consumed), resolving surrogate pairs.
    fn unicode_escape(&mut self) -> Result<char, String> {
        let cp = self.hex4()?;
        if (0xD800..=0xDBFF).contains(&cp) {
            // High surrogate → require a following `\uXXXX` low surrogate.
            if self.peek() == Some(b'\\') && self.b.get(self.i + 1) == Some(&b'u') {
                self.i += 2;
                let lo = self.hex4()?;
                if (0xDC00..=0xDFFF).contains(&lo) {
                    let c = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                    return char::from_u32(c).ok_or_else(|| "bad surrogate pair".into());
                }
                return Err("invalid low surrogate".into());
            }
            return Err("missing low surrogate".into());
        }
        char::from_u32(cp).ok_or_else(|| "invalid code point".into())
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-') {
                self.i += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| "bad number bytes")?;
        s.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| format!("invalid number '{s}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_scalars_and_integers() {
        assert_eq!(Json::Null.to_compact(), "null");
        assert_eq!(Json::Bool(true).to_compact(), "true");
        assert_eq!(Json::Num(42.0).to_compact(), "42"); // integral → no ".0"
        assert_eq!(Json::Num(-7.5).to_compact(), "-7.5");
        assert_eq!(Json::from("hi").to_compact(), "\"hi\"");
    }

    #[test]
    fn escapes_control_and_quotes() {
        let s = Json::from("a\"b\\c\n\té");
        // Quotes/backslash/newline/tab escaped; é (non-control) stays literal.
        assert_eq!(s.to_compact(), "\"a\\\"b\\\\c\\n\\té\"");
    }

    #[test]
    fn pretty_prints_nested() {
        let j = Json::obj(vec![
            ("n", 1u32.into()),
            ("xs", Json::Arr(vec![1u32.into(), 2u32.into()])),
        ]);
        assert_eq!(
            j.to_pretty(),
            "{\n  \"n\": 1,\n  \"xs\": [\n    1,\n    2\n  ]\n}"
        );
        assert_eq!(Json::Arr(vec![]).to_pretty(), "[]");
        assert_eq!(Json::obj(vec![]).to_pretty(), "{}");
    }

    #[test]
    fn roundtrips_via_parse() {
        let original = Json::obj(vec![
            ("s", "café — \"quoted\"".into()),
            ("b", true.into()),
            ("nil", Json::Null),
            ("num", Json::Num(-3.25)),
            (
                "arr",
                Json::Arr(vec![1u32.into(), Json::obj(vec![("k", "v".into())])]),
            ),
        ]);
        let text = original.to_pretty();
        assert_eq!(Json::parse(&text).unwrap(), original);
    }

    #[test]
    fn parses_unicode_escapes_and_surrogates() {
        assert_eq!(Json::parse(r#""é""#).unwrap().as_str(), Some("é"));
        // U+1F600 as a surrogate pair.
        assert_eq!(Json::parse(r#""😀""#).unwrap().as_str(), Some("😀"));
    }

    #[test]
    fn parses_and_reads_object() {
        let j = Json::parse(r#" { "a": 1, "b": [true, null], "c": "x" } "#).unwrap();
        assert_eq!(j.get("a").and_then(Json::as_i64), Some(1));
        assert_eq!(
            j.get("b").and_then(Json::as_array).map(|a| a.len()),
            Some(2)
        );
        assert_eq!(j.get("c").and_then(Json::as_str), Some("x"));
        assert!(j.get("missing").is_none());
    }

    #[test]
    fn rejects_malformed() {
        assert!(Json::parse("").is_err());
        assert!(Json::parse("{").is_err());
        assert!(Json::parse("[1,]").is_err());
        assert!(Json::parse(r#"{"k" 1}"#).is_err());
        assert!(Json::parse("1 2").is_err()); // trailing data
        assert!(Json::parse(r#""\uZZZZ""#).is_err());
    }
}
