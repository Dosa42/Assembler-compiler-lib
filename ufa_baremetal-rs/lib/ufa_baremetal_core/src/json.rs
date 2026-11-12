//! Minimal hand-rolled JSON writer matching Python's
//! `json.dump(doc, f, indent=2)` formatting exactly: 2-space indents,
//! `": "` key separator, non-ASCII characters escaped as `\uXXXX`
//! (Python's `ensure_ascii=True` default, which `cli.py`'s `_emit_json`
//! never overrides). Used only for the CLI's `--json` output.
//!
//! Not a general JSON parser/writer — this crate hand-rolls this one
//! fixed, small schema rather than depending on `serde_json`, continuing
//! the "stdlib only" discipline described in the core crate's
//! `Cargo.toml`. The exact formatting (including empty-container and
//! non-ASCII-escaping behavior) was verified against real
//! `json.dump` output before being written here — see `tests` below.

pub enum Json {
    Null,
    Int(i64),
    UInt(u64),
    Str(String),
    Array(Vec<Json>),
    /// Preserves insertion order (a `Vec`, not a map) — matching Python
    /// dict's insertion-order guarantee, which `_emit_json` relies on for
    /// stable key ordering in the output.
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }

    pub fn opt_str(s: Option<String>) -> Json {
        match s {
            Some(s) => Json::Str(s),
            None => Json::Null,
        }
    }

    pub fn opt_u32(n: Option<u32>) -> Json {
        match n {
            Some(n) => Json::UInt(n as u64),
            None => Json::Null,
        }
    }

    pub fn to_pretty_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Int(n) => out.push_str(&n.to_string()),
            Json::UInt(n) => out.push_str(&n.to_string()),
            Json::Str(s) => out.push_str(&escape_json_string(s)),
            Json::Array(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                let inner = indent + 2;
                let pad = " ".repeat(inner);
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&pad);
                    item.write(out, inner);
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(indent));
                out.push(']');
            }
            Json::Object(pairs) => {
                if pairs.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                let inner = indent + 2;
                let pad = " ".repeat(inner);
                for (i, (k, v)) in pairs.iter().enumerate() {
                    out.push_str(&pad);
                    out.push_str(&escape_json_string(k));
                    out.push_str(": ");
                    v.write(out, inner);
                    if i + 1 < pairs.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&" ".repeat(indent));
                out.push('}');
            }
        }
    }
}

/// Matches Python's `ensure_ascii=True` string escaping: standard JSON
/// escapes for quote/backslash/control chars, and every character
/// outside printable ASCII (`0x20`..=`0x7E`) escaped as `\uXXXX` (a
/// surrogate pair for code points above the BMP).
fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
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
            c if (c as u32) <= 0x7E => out.push(c),
            c => {
                let cp = c as u32;
                if cp > 0xFFFF {
                    let cp2 = cp - 0x10000;
                    let high = 0xD800 + (cp2 >> 10);
                    let low = 0xDC00 + (cp2 & 0x3FF);
                    out.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                } else {
                    out.push_str(&format!("\\u{cp:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_python_json_dump_indent_2_byte_for_byte() {
        // Same document shape used to empirically capture the expected
        // output from Python's own `json.dump(doc, sys.stdout, indent=2)`
        // before writing this module.
        let doc = Json::Object(vec![
            ("tool_version".into(), Json::str("x")),
            ("generated_at".into(), Json::str("2026-01-01T00:00:00.000000+00:00")),
            (
                "inputs".into(),
                Json::Array(vec![Json::Object(vec![
                    ("name".into(), Json::str("a.s")),
                    ("path".into(), Json::str("/x/a.s")),
                    ("sha256".into(), Json::str("abc")),
                    ("bytes".into(), Json::UInt(3)),
                ])]),
            ),
            (
                "verdicts".into(),
                Json::Array(vec![Json::Object(vec![
                    ("status".into(), Json::str("FAIL")),
                    ("k_class".into(), Json::str("K01")),
                    ("file".into(), Json::str("a.s")),
                    ("function".into(), Json::str("f")),
                    ("line_no".into(), Json::UInt(4)),
                    ("diagnostic".into(), Json::str("em—dash § sign × mult")),
                    ("suggested_fix".into(), Json::str("")),
                ])]),
            ),
            (
                "summary".into(),
                Json::Object(vec![
                    ("PASS".into(), Json::UInt(0)),
                    ("FAIL".into(), Json::UInt(1)),
                    ("NOT_APPLICABLE".into(), Json::UInt(0)),
                ]),
            ),
            ("empty_obj".into(), Json::Object(vec![])),
            ("empty_arr".into(), Json::Array(vec![])),
        ]);

        let expected = "{\n  \"tool_version\": \"x\",\n  \"generated_at\": \"2026-01-01T00:00:00.000000+00:00\",\n  \"inputs\": [\n    {\n      \"name\": \"a.s\",\n      \"path\": \"/x/a.s\",\n      \"sha256\": \"abc\",\n      \"bytes\": 3\n    }\n  ],\n  \"verdicts\": [\n    {\n      \"status\": \"FAIL\",\n      \"k_class\": \"K01\",\n      \"file\": \"a.s\",\n      \"function\": \"f\",\n      \"line_no\": 4,\n      \"diagnostic\": \"em\\u2014dash \\u00a7 sign \\u00d7 mult\",\n      \"suggested_fix\": \"\"\n    }\n  ],\n  \"summary\": {\n    \"PASS\": 0,\n    \"FAIL\": 1,\n    \"NOT_APPLICABLE\": 0\n  },\n  \"empty_obj\": {},\n  \"empty_arr\": []\n}";

        assert_eq!(doc.to_pretty_string(), expected);
    }

    #[test]
    fn null_for_absent_optional_fields() {
        assert!(matches!(Json::opt_str(None), Json::Null));
        assert!(matches!(Json::opt_u32(None), Json::Null));
        assert!(matches!(Json::opt_str(Some("x".into())), Json::Str(s) if s == "x"));
    }
}
