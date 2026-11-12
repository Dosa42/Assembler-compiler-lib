//! A minimal, hand-rolled JSON value type, parser, and serializer.
//!
//! `ufa_baremetal_core::json` already hand-rolls a JSON *writer* matching
//! Python's `json.dump(..., indent=2)` byte-for-byte for the `--json` CLI
//! schema, but it's write-only by design (see its own doc comment) and
//! tied to that one fixed schema. Speaking JSON-RPC to an LSP client needs
//! both directions -- parsing arbitrary incoming request/notification
//! bodies as well as writing responses -- so this is a second, separate,
//! general-purpose implementation living in this crate rather than a
//! change to the core crate's schema-specific one. `ufa_baremetal_core`'s
//! zero-dependency, I/O-free, single-schema JSON discipline is untouched
//! by this file existing.
//!
//! Deliberately not a `serde_json` replacement: no derive macros, no
//! streaming, no arbitrary-precision numbers (every LSP integer this
//! server touches -- request ids, line/character numbers, versions --
//! fits comfortably in an `f64`'s 53-bit exact-integer range). Just
//! enough JSON to parse and emit valid JSON-RPC / LSP messages.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    /// Insertion-ordered (a `Vec`, not a map), matching
    /// `ufa_baremetal_core::json::Json::Object`'s own choice -- order is
    /// irrelevant to JSON-RPC correctness (field lookup is always by
    /// name) but keeps serialized output stable and readable in tests.
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    pub fn int(n: i64) -> Value {
        Value::Num(n as f64)
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parses one complete JSON document from `input`. Errors on trailing
/// non-whitespace data after the value, matching `serde_json::from_str`'s
/// behavior (a JSON-RPC message body is exactly one JSON value, nothing
/// more) rather than silently accepting `{}garbage`.
pub fn parse(input: &str) -> Result<Value, ParseError> {
    let mut p = Parser { chars: input.chars().collect(), pos: 0 };
    let v = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.chars.len() {
        return Err(ParseError(format!("trailing data after JSON value at char {}", p.pos)));
    }
    Ok(v)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, c: char) -> Result<(), ParseError> {
        match self.bump() {
            Some(x) if x == c => Ok(()),
            other => Err(ParseError(format!("expected '{c}', found {other:?} at char {}", self.pos))),
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => self.parse_string().map(Value::Str),
            Some('t') => self.parse_literal("true", Value::Bool(true)),
            Some('f') => self.parse_literal("false", Value::Bool(false)),
            Some('n') => self.parse_literal("null", Value::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            other => Err(ParseError(format!("unexpected character {other:?} at char {}", self.pos))),
        }
    }

    fn parse_literal(&mut self, lit: &str, value: Value) -> Result<Value, ParseError> {
        for expected in lit.chars() {
            match self.bump() {
                Some(c) if c == expected => {}
                other => {
                    return Err(ParseError(format!(
                        "expected literal '{lit}', found {other:?} at char {}",
                        self.pos
                    )))
                }
            }
        }
        Ok(value)
    }

    fn parse_object(&mut self) -> Result<Value, ParseError> {
        self.expect('{')?;
        let mut pairs = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Value::Obj(pairs));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(':')?;
            let value = self.parse_value()?;
            pairs.push((key, value));
            self.skip_ws();
            match self.bump() {
                Some(',') => continue,
                Some('}') => break,
                other => {
                    return Err(ParseError(format!(
                        "expected ',' or '}}' in object, found {other:?} at char {}",
                        self.pos
                    )))
                }
            }
        }
        Ok(Value::Obj(pairs))
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(Value::Arr(items));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_ws();
            match self.bump() {
                Some(',') => continue,
                Some(']') => break,
                other => {
                    return Err(ParseError(format!(
                        "expected ',' or ']' in array, found {other:?} at char {}",
                        self.pos
                    )))
                }
            }
        }
        Ok(Value::Arr(items))
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(ParseError("unterminated string".into())),
                Some('"') => break,
                Some('\\') => match self.bump() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('/') => out.push('/'),
                    Some('b') => out.push('\u{08}'),
                    Some('f') => out.push('\u{0C}'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('u') => {
                        let cp = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&cp) {
                            // High surrogate: a well-formed \uXXXX\uYYYY
                            // surrogate pair must follow immediately.
                            if self.bump() != Some('\\') || self.bump() != Some('u') {
                                return Err(ParseError(
                                    "expected low surrogate \\u escape after high surrogate".into(),
                                ));
                            }
                            let low = self.parse_hex4()?;
                            if !(0xDC00..=0xDFFF).contains(&low) {
                                return Err(ParseError("invalid low surrogate".into()));
                            }
                            let c = 0x10000u32 + (((cp - 0xD800) as u32) << 10) + (low - 0xDC00) as u32;
                            out.push(char::from_u32(c).ok_or_else(|| ParseError("invalid surrogate pair".into()))?);
                        } else {
                            out.push(
                                char::from_u32(cp as u32)
                                    .ok_or_else(|| ParseError("invalid \\u escape".into()))?,
                            );
                        }
                    }
                    other => return Err(ParseError(format!("invalid escape sequence {other:?}"))),
                },
                Some(c) => out.push(c),
            }
        }
        Ok(out)
    }

    fn parse_hex4(&mut self) -> Result<u16, ParseError> {
        let mut v: u16 = 0;
        for _ in 0..4 {
            let c = self.bump().ok_or_else(|| ParseError("unexpected end of \\u escape".into()))?;
            let d = c.to_digit(16).ok_or_else(|| ParseError(format!("invalid hex digit '{c}' in \\u escape")))?;
            v = v * 16 + d as u16;
        }
        Ok(v)
    }

    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.bump();
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.bump();
        }
        if self.peek() == Some('.') {
            self.bump();
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.bump();
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.bump();
            }
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse::<f64>().map(Value::Num).map_err(|e| ParseError(format!("invalid number '{s}': {e}")))
    }
}

/// Serializes `v` as compact JSON (no pretty-printing -- unlike the
/// `--json` CLI schema, nothing reads this output by eye; a client's own
/// LSP library parses it). Returned as `String` so the caller can compute
/// its exact UTF-8 byte length for the `Content-Length` header.
pub fn to_string(v: &Value) -> String {
    let mut out = String::new();
    write_value(v, &mut out);
    out
}

fn write_value(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => {
            // Every number this server ever writes (ids, line/character
            // offsets, versions, error codes) is an exact integer; render
            // as one rather than risking e.g. `4` becoming `4.0`, which
            // some strict JSON-RPC clients balk at for id fields.
            if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                out.push_str(&(*n as i64).to_string());
            } else {
                out.push_str(&n.to_string());
            }
        }
        Value::Str(s) => write_escaped_string(s, out),
        Value::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Obj(pairs) => {
            out.push('{');
            for (i, (k, val)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_escaped_string(k, out);
                out.push(':');
                write_value(val, out);
            }
            out.push('}');
        }
    }
}

fn write_escaped_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            // Unlike core's json.rs (which must match Python's
            // `ensure_ascii=True`), non-ASCII is left as raw UTF-8 here:
            // still perfectly valid JSON, and every LSP client speaks
            // UTF-8 over stdio by default (the base protocol's charset).
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_primitive_type() {
        for (src, expected) in [
            ("null", Value::Null),
            ("true", Value::Bool(true)),
            ("false", Value::Bool(false)),
            ("0", Value::Num(0.0)),
            ("-42", Value::Num(-42.0)),
            ("3.5", Value::Num(3.5)),
            ("1e3", Value::Num(1000.0)),
            ("-2.5E-1", Value::Num(-0.25)),
            ("\"hello\"", Value::Str("hello".into())),
        ] {
            assert_eq!(parse(src).unwrap(), expected, "parsing {src:?}");
        }
    }

    #[test]
    fn parses_nested_objects_and_arrays() {
        let v = parse(r#"{"a": [1, 2, {"b": null}], "c": true}"#).unwrap();
        assert_eq!(v.get("a").unwrap().as_array().unwrap().len(), 3);
        assert_eq!(v.get("a").unwrap().as_array().unwrap()[0], Value::Num(1.0));
        assert_eq!(v.get("a").unwrap().as_array().unwrap()[2].get("b").unwrap(), &Value::Null);
        assert_eq!(v.get("c").unwrap(), &Value::Bool(true));
    }

    #[test]
    fn parses_string_escapes_including_surrogate_pairs() {
        let v = parse(r#""line1\nline2\ttabbed\"quoted\"""#).unwrap();
        assert_eq!(v, Value::Str("line1\nline2\ttabbed\"quoted\"".into()));

        // U+1F600 GRINNING FACE, encoded as a UTF-16 surrogate pair --
        // exactly the shape a real editor sends for emoji in file paths
        // or comments.
        let v = parse(r#""\ud83d\ude00""#).unwrap();
        assert_eq!(v, Value::Str("\u{1F600}".into()));
    }

    #[test]
    fn ignores_whitespace_between_tokens() {
        let v = parse("  {  \"a\"  :  1  ,  \"b\"  :  2  }  ").unwrap();
        assert_eq!(v.get("a").unwrap(), &Value::Num(1.0));
        assert_eq!(v.get("b").unwrap(), &Value::Num(2.0));
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse("{}garbage").is_err());
        assert!(parse("1 2").is_err());
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(parse("{\"a\":}").is_err());
        assert!(parse("[1,2,]").is_err());
        assert!(parse("\"unterminated").is_err());
        assert!(parse("nul").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn serialize_round_trips_through_parse() {
        let original = Value::Obj(vec![
            ("jsonrpc".into(), Value::str("2.0")),
            ("id".into(), Value::int(7)),
            (
                "result".into(),
                Value::Obj(vec![
                    ("ok".into(), Value::Bool(true)),
                    ("items".into(), Value::Arr(vec![Value::str("a"), Value::Null, Value::int(-3)])),
                    ("note".into(), Value::str("quote\" and \\ and newline\n and unicode: café")),
                ]),
            ),
        ]);
        let text = to_string(&original);
        let reparsed = parse(&text).unwrap();
        assert_eq!(reparsed, original);
    }

    #[test]
    fn serializes_integers_without_a_trailing_decimal_point() {
        assert_eq!(to_string(&Value::int(4)), "4");
        assert_eq!(to_string(&Value::Num(4.0)), "4");
        assert_eq!(to_string(&Value::Num(4.5)), "4.5");
    }
}
