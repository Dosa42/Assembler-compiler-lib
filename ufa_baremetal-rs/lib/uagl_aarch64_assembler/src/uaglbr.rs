//! Embedded UAGL Braille Latin source loader — a faithful port of the
//! `_UAGLBR_*` constants and `_uaglbr_canonical_json`,
//! `_uaglbr_validate_digest`, `_uaglbr_unpack_bitmap`, `decode_uaglbr_bytes`,
//! `_canonical_uagl_source_name`, and `load_uagl_source`.
//!
//! `decode_uaglbr_bytes` validates an *untrusted* container end to end, so
//! the metadata is parsed generically (`serde_json::Value`) rather than
//! directly into a fixed-shape struct: several of the original's checks
//! (the canonical-JSON byte comparison, the int/bool type checks) only
//! make sense — and only match Python's `json.loads` type distinctions —
//! if a field can still be observed as "the wrong JSON type" at the point
//! it's examined, exactly as the Python `dict` produced by `json.loads`
//! would allow.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

const UAGLBR_MAGIC: &[u8] = b"UAGL-BRAILLE-LATIN/1\n";

/// Whether `data` starts with the `.uaglbr` container magic. Exposed so
/// `assemble_uagl_bytes` (the Rust equivalent of the Python
/// `assemble_uagl` overload that accepts `bytes`) can branch the same way
/// the original's `isinstance(uagl_src, bytes)` arm does.
pub fn has_uaglbr_magic(data: &[u8]) -> bool {
    data.starts_with(UAGLBR_MAGIC)
}
const UAGLBR_FORMAT: &str = "UAGL-BRAILLE-LATIN";
const UAGLBR_VERSION: i64 = 1;
const UAGLBR_SCHEMA: &[&str] = &[
    "bitmap_b64",
    "char_count",
    "encoding",
    "format",
    "payload_sha256",
    "payload_size",
    "source_sha256",
    "source_size",
    "target_count",
    "version",
];

const LOWER_PATTERNS: &[(char, u32)] = &[
    ('a', 0x01), ('b', 0x03), ('c', 0x09), ('d', 0x19), ('e', 0x11),
    ('f', 0x0B), ('g', 0x1B), ('h', 0x13), ('i', 0x0A), ('j', 0x1A),
    ('k', 0x05), ('l', 0x07), ('m', 0x0D), ('n', 0x1D), ('o', 0x15),
    ('p', 0x0F), ('q', 0x1F), ('r', 0x17), ('s', 0x0E), ('t', 0x1E),
    ('u', 0x25), ('v', 0x27), ('w', 0x3A), ('x', 0x2D), ('y', 0x3D),
    ('z', 0x35),
];

const DIGIT_TO_LETTER: &[(char, char)] = &[
    ('1', 'a'), ('2', 'b'), ('3', 'c'), ('4', 'd'), ('5', 'e'),
    ('6', 'f'), ('7', 'g'), ('8', 'h'), ('9', 'i'), ('0', 'j'),
];

static ENCODE_MAP: Lazy<HashMap<char, char>> = Lazy::new(|| {
    let mut m = HashMap::new();
    for &(letter, pattern) in LOWER_PATTERNS {
        m.insert(letter, char::from_u32(0x2800 + pattern).unwrap());
        m.insert(
            letter.to_ascii_uppercase(),
            char::from_u32(0x2800 + pattern + 0x40).unwrap(),
        );
    }
    for &(digit, letter) in DIGIT_TO_LETTER {
        let pattern = LOWER_PATTERNS.iter().find(|&&(l, _)| l == letter).unwrap().1;
        m.insert(digit, char::from_u32(0x2800 + pattern + 0x80).unwrap());
    }
    m
});

static DECODE_MAP: Lazy<HashMap<char, char>> =
    Lazy::new(|| ENCODE_MAP.iter().map(|(&src, &enc)| (enc, src)).collect());

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

fn canonical_json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) > 0x7E => {
                let cp = c as u32;
                if cp > 0xFFFF {
                    let cp2 = cp - 0x10000;
                    let hi = 0xD800 + (cp2 >> 10);
                    let lo = 0xDC00 + (cp2 & 0x3FF);
                    out.push_str(&format!("\\u{:04x}\\u{:04x}", hi, lo));
                } else {
                    out.push_str(&format!("\\u{:04x}", cp));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn canonical_json_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    if let Some(f) = n.as_f64() {
        return format!("{}", f);
    }
    n.to_string()
}

/// Re-serialize a generic JSON value exactly the way Python's
/// `json.dumps(doc, ensure_ascii=True, sort_keys=True, separators=(",",":"))`
/// would. `serde_json::Map` (the `Value::Object` backing store) is a
/// `BTreeMap` by default (no `preserve_order` feature enabled in this
/// crate), so iterating it already yields keys in sorted order — matching
/// `sort_keys=True` for free.
fn canonical_json_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        serde_json::Value::Number(n) => canonical_json_number(n),
        serde_json::Value::String(s) => canonical_json_string(s),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(canonical_json_value).collect();
            format!("[{}]", items.join(","))
        }
        serde_json::Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}:{}", canonical_json_string(k), canonical_json_value(v)))
                .collect();
            format!("{{{}}}", items.join(","))
        }
    }
}

/// Parse the way Python's permissive `int(s, 16)` would for a digest
/// format check: optional surrounding whitespace, optional sign, optional
/// `0x`/`0X` prefix, then one or more hex digits and nothing else.
fn python_int_base16_parses(s: &str) -> bool {
    let t = s.trim();
    let t = t.strip_prefix('-').or_else(|| t.strip_prefix('+')).unwrap_or(t);
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    !t.is_empty() && t.chars().all(|c| c.is_ascii_hexdigit())
}

fn validate_digest(metadata: &serde_json::Value, key: &str) -> Result<(), String> {
    let value = metadata.get(key).and_then(|v| v.as_str());
    let value = match value {
        Some(v) if v.chars().count() == 64 => v,
        _ => return Err(format!("invalid UAGL Braille {}", key)),
    };
    if !python_int_base16_parses(value) {
        return Err(format!("invalid UAGL Braille {}", key));
    }
    if value != value.to_lowercase() {
        return Err(format!("non-canonical UAGL Braille {}", key));
    }
    Ok(())
}

fn unpack_bitmap(bitmap: &[u8], char_count: i64) -> Result<Vec<bool>, String> {
    let expected = (char_count + 7) / 8;
    if bitmap.len() as i64 != expected {
        return Err(format!(
            "UAGL Braille bitmap length mismatch: expected {}, got {}",
            expected,
            bitmap.len()
        ));
    }
    if char_count != 0 && char_count % 8 != 0 {
        let used = (1u32 << (char_count % 8)) - 1;
        let last = *bitmap.last().unwrap() as u32;
        if last & (!used & 0xFF) != 0 {
            return Err("UAGL Braille bitmap tail bits are non-zero".to_string());
        }
    }
    Ok((0..char_count)
        .map(|index| {
            let byte = bitmap[(index / 8) as usize] as u32;
            (byte & (1 << (index % 8))) != 0
        })
        .collect())
}

fn nonneg_int(metadata: &serde_json::Value, key: &str) -> Result<i64, String> {
    match metadata.get(key).and_then(|v| v.as_i64()) {
        Some(n) if n >= 0 => Ok(n),
        _ => Err(format!("invalid UAGL Braille {}", key)),
    }
}

/// Strictly restore canonical UAGL source bytes from one .uaglbr container.
pub fn decode_uaglbr_bytes(container: &[u8]) -> Result<Vec<u8>, String> {
    if !container.starts_with(UAGLBR_MAGIC) {
        return Err("invalid UAGL Braille container magic".to_string());
    }
    let remainder = &container[UAGLBR_MAGIC.len()..];
    let split_at = remainder
        .iter()
        .position(|&b| b == b'\n')
        .ok_or_else(|| "UAGL Braille metadata line is missing".to_string())?;
    let metadata_line = &remainder[..split_at];
    let payload = &remainder[split_at + 1..];

    let metadata_str = std::str::from_utf8(metadata_line)
        .ok()
        .filter(|s| s.is_ascii())
        .ok_or_else(|| "invalid UAGL Braille metadata".to_string())?;
    let metadata: serde_json::Value =
        serde_json::from_str(metadata_str).map_err(|_| "invalid UAGL Braille metadata".to_string())?;
    let obj = metadata
        .as_object()
        .ok_or_else(|| "UAGL Braille metadata must be an object".to_string())?;

    let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
    keys.sort_unstable();
    let mut schema_sorted: Vec<&str> = UAGLBR_SCHEMA.to_vec();
    schema_sorted.sort_unstable();
    if keys != schema_sorted {
        return Err("UAGL Braille metadata schema mismatch".to_string());
    }

    if metadata.get("format").and_then(|v| v.as_str()) != Some(UAGLBR_FORMAT)
        || metadata.get("version").and_then(|v| v.as_f64()) != Some(UAGLBR_VERSION as f64)
    {
        return Err("unsupported UAGL Braille format or version".to_string());
    }
    if metadata.get("encoding").and_then(|v| v.as_str()) != Some("utf-8") {
        return Err("unsupported UAGL Braille source encoding".to_string());
    }
    if metadata_line != canonical_json_value(&metadata).as_bytes() {
        return Err("UAGL Braille metadata is not canonical".to_string());
    }

    let char_count = nonneg_int(&metadata, "char_count")?;
    let payload_size = nonneg_int(&metadata, "payload_size")?;
    let source_size = nonneg_int(&metadata, "source_size")?;
    let target_count = nonneg_int(&metadata, "target_count")?;
    if target_count > char_count {
        return Err("UAGL Braille target count exceeds character count".to_string());
    }
    validate_digest(&metadata, "payload_sha256")?;
    validate_digest(&metadata, "source_sha256")?;
    let payload_sha256 = metadata.get("payload_sha256").and_then(|v| v.as_str()).unwrap();
    let source_sha256 = metadata.get("source_sha256").and_then(|v| v.as_str()).unwrap();

    if metadata.get("bitmap_b64").and_then(|v| v.as_str()).is_none() {
        return Err("invalid UAGL Braille bitmap base64".to_string());
    }
    let bitmap_b64 = metadata.get("bitmap_b64").and_then(|v| v.as_str()).unwrap();

    if payload.len() as i64 != payload_size {
        return Err("UAGL Braille payload size mismatch".to_string());
    }
    if sha256_hex(payload) != payload_sha256 {
        return Err("UAGL Braille payload SHA-256 mismatch".to_string());
    }
    let payload_text =
        std::str::from_utf8(payload).map_err(|_| "UAGL Braille payload is not valid UTF-8".to_string())?;
    if payload_text.chars().count() as i64 != char_count {
        return Err("UAGL Braille character count mismatch".to_string());
    }

    let bitmap = B64
        .decode(bitmap_b64)
        .map_err(|_| "invalid UAGL Braille bitmap base64".to_string())?;
    if B64.encode(&bitmap) != bitmap_b64 {
        return Err("UAGL Braille bitmap base64 is not canonical".to_string());
    }
    let flags = unpack_bitmap(&bitmap, payload_text.chars().count() as i64)?;
    if flags.iter().filter(|&&b| b).count() as i64 != target_count {
        return Err("UAGL Braille target count mismatch".to_string());
    }

    let mut restored = String::new();
    for (index, (ch, marked)) in payload_text.chars().zip(flags.iter()).enumerate() {
        if !marked {
            restored.push(ch);
            continue;
        }
        match DECODE_MAP.get(&ch) {
            Some(&source_char) => restored.push(source_char),
            None => return Err(format!("invalid UAGL Braille cell at marked position {}", index)),
        }
    }
    let source = restored.into_bytes();
    if source.len() as i64 != source_size {
        return Err("UAGL Braille restored source size mismatch".to_string());
    }
    if sha256_hex(&source) != source_sha256 {
        return Err("UAGL Braille restored source SHA-256 mismatch".to_string());
    }
    Ok(source)
}

fn canonical_uagl_source_name(path: &Path, was_uaglbr: bool) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if !was_uaglbr {
        return name;
    }
    let name = name.strip_suffix(".uaglbr").unwrap_or(&name).to_string();
    if name.ends_with(".uagl") {
        name
    } else {
        format!("{}.uagl", name)
    }
}

/// Load raw .uagl or auto-detect and decode canonical .uaglbr bytes.
/// Returns (source_text, canonical_name, was_uaglbr).
pub fn load_uagl_source(src_path: &str) -> Result<(String, String, bool), String> {
    let path = Path::new(src_path);
    let raw = std::fs::read(path).map_err(|e| e.to_string())?;
    let has_magic = raw.starts_with(UAGLBR_MAGIC);
    let file_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if file_name.ends_with(".uaglbr") && !has_magic {
        return Err("invalid UAGL Braille container magic".to_string());
    }
    let was_uaglbr = has_magic;
    let source_bytes = if was_uaglbr {
        decode_uaglbr_bytes(&raw)?
    } else {
        raw
    };
    let source_text = if was_uaglbr {
        String::from_utf8(source_bytes).map_err(|e| e.to_string())?
    } else {
        String::from_utf8_lossy(&source_bytes).to_string()
    };
    Ok((source_text, canonical_uagl_source_name(path, was_uaglbr), was_uaglbr))
}
