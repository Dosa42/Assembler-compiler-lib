//! Small parsing and string-manipulation helpers shared by the expression
//! codegen and the main generator. A faithful port of `parse_params`,
//! `parse_condition`, `_looks_like_type`, `_strip_outer_parens`,
//! `_has_top_level`, `_split_binop`, `_decode_uagl_string`,
//! `_aarch64_addsub_imm_lines`, `_fp_ldr_lines`, and `_fp_str_lines`.
//!
//! Several of these scan a string character-by-character and slice by
//! character position (Python strings index by Unicode code point, and
//! these expressions routinely mix ASCII operators with multi-byte UAGL
//! symbols like `─`, `△`, `○`). To keep index arithmetic identical to the
//! original we operate on `Vec<char>` rather than on raw byte slices.

use once_cell::sync::Lazy;
use regex::Regex;

/// `_looks_like_type`'s char class: `^@?[A-Za-z_][\w]*(?:\[\d+\])?$`.
static TYPE_TOKEN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^@?[A-Za-z_][\w]*(?:\[\d+\])?$").unwrap()
});

const BUILTIN_TYPES: &[&str] = &[
    "UINT8", "INT8", "UINT16", "INT16", "UINT32", "INT32", "UINT64", "INT64",
    "U8", "S8", "U16", "S16", "U32", "S32", "U64", "S64",
    "FLOAT", "DOUBLE", "BOOL", "CHAR", "VOID", "PTR", "GVector",
];

/// Character-index slice matching Python's `s[1:-1]` (drop exactly one
/// leading and one trailing *character*, not byte — several UAGL
/// expressions mix ASCII with multi-byte symbols).
pub fn drop_first_last_char(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 2 {
        return String::new();
    }
    chars[1..chars.len() - 1].iter().collect()
}

/// Parse a numeric literal the way Python's `int(s, 0)` would: `0x`/`0b`
/// prefixed hex/binary, or plain decimal with NO ambiguous leading zero
/// (Python's base-0 rule rejects `"007"`, only `"0"` itself is allowed to
/// start with `0`). Returns `None` on any parse failure — the direct
/// analog of the `except (ValueError, TypeError): pass` guards that wrap
/// nearly every call site of `int(val, 0)` in the original. A leading `-`
/// is accepted (matching the original's separate sign-stripping call sites
/// that pass an already-stripped numeric string here).
pub fn try_parse_base0(s: &str) -> Option<i128> {
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let n: i128 = if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i128::from_str_radix(rest, 16).ok()?
    } else if let Some(rest) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        i128::from_str_radix(rest, 2).ok()?
    } else if let Some(rest) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) {
        i128::from_str_radix(rest, 8).ok()?
    } else {
        if s.len() > 1 && s.starts_with('0') {
            return None;
        }
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse::<i128>().ok()?
    };
    Some(if neg { -n } else { n })
}

/// A type token is one of: built-in primitives, user struct names
/// (uppercase-leading, not all-caps), pointer-to-anything (`@T`), or a
/// sized array (`T[N]`).
pub fn looks_like_type(tok: &str) -> bool {
    if !TYPE_TOKEN_RE.is_match(tok) {
        return false;
    }
    if tok.starts_with('@') {
        return true;
    }
    if BUILTIN_TYPES.contains(&tok) {
        return true;
    }
    if tok.contains('[') {
        return true;
    }
    // User-defined struct: starts with an uppercase letter, but is not
    // itself an all-caps token (which would look like a bare constant).
    let first_upper = tok.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
    let all_upper = !tok.is_empty() && tok.chars().all(|c| !c.is_lowercase());
    first_upper && !all_upper
}

/// Parse `'p1 ─ T1 ─ p2 ─ T2 ─ RETTYPE'` -> `([(p1,T1),(p2,T2)], RETTYPE)`.
/// The last token is always the return type.
pub fn parse_params(raw: &str) -> (Vec<(String, String)>, String) {
    static SEP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*─\s*").unwrap());
    let parts: Vec<String> = SEP_RE
        .split(raw)
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return (Vec::new(), "VOID".to_string());
    }
    let ret_type = parts[parts.len() - 1].clone();
    let body = &parts[..parts.len() - 1];
    let mut params = Vec::new();
    let mut i = 0usize;
    while i < body.len() {
        let name = body[i].clone();
        if i + 1 < body.len() && looks_like_type(&body[i + 1]) {
            params.push((name, body[i + 1].clone()));
            i += 2;
        } else {
            params.push((name, "UINT64".to_string()));
            i += 1;
        }
    }
    (params, ret_type)
}

/// Parse `'a > b'`, `'a △ b'`, `'a < b'`, etc. Returns `(lhs, op, rhs)`.
/// Order matters: two-character operators must be checked before their
/// one-character prefixes (`>=` before `>`) or splitting would cut the
/// wrong place.
pub fn parse_condition(cond: &str) -> (String, String, String) {
    const OPS: &[&str] = &[">=", "<=", "△", "╱", "∧", "∨", ">", "<", "==", "!="];
    for op in OPS {
        if cond.contains(op) {
            let mut parts = cond.splitn(2, op);
            let lhs = parts.next().unwrap_or("").trim().to_string();
            let rhs = parts.next().unwrap_or("").trim().to_string();
            return (lhs, op.to_string(), rhs);
        }
    }
    (cond.trim().to_string(), "╱".to_string(), "0".to_string())
}

/// Strip a single layer of balanced outer parens, repeatedly.
pub fn strip_outer_parens(s: &str) -> String {
    let mut chars: Vec<char> = s.trim().chars().collect();
    loop {
        if chars.first() != Some(&'(') || chars.last() != Some(&')') {
            break;
        }
        let mut depth: i64 = 0;
        let mut balanced_at_end = true;
        let n = chars.len();
        for (i, &c) in chars.iter().enumerate() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
            }
            if depth == 0 && i < n - 1 {
                balanced_at_end = false;
                break;
            }
        }
        if !balanced_at_end {
            break;
        }
        chars = chars[1..n - 1]
            .iter()
            .collect::<String>()
            .trim()
            .chars()
            .collect();
    }
    chars.into_iter().collect()
}

/// True if `ch` appears at paren+bracket depth 0 in `s`.
pub fn has_top_level(s: &str, ch: char) -> bool {
    let mut paren = 0i64;
    let mut bracket = 0i64;
    for c in s.chars() {
        match c {
            '(' => paren += 1,
            ')' => paren -= 1,
            '[' => bracket += 1,
            ']' => bracket -= 1,
            _ if paren == 0 && bracket == 0 && c == ch => return true,
            _ => {}
        }
    }
    false
}

/// Precedence classes, lowest first (mirrors `_BINOP_PRECEDENCE`).
const BINOP_PRECEDENCE: &[&[&str]] = &[
    &["|"],
    &["^"],
    &["&"],
    &["<<", ">>"],
    &["+", "-"],
    &["*", "/", "%"],
];

/// Find the lowest-precedence top-level binary operator in `s`.
/// Returns `Some((lhs, op, rhs))` or `None`. Tracks `()`/`[]` depth and
/// stops scanning at the leftmost top-level `○` (call) — everything from
/// the call's function name onward is treated as an atomic call
/// expression, so operators inside call arguments are not picked up.
pub fn split_binop(s: &str) -> Option<(String, String, String)> {
    let stripped = strip_outer_parens(s);
    let chars: Vec<char> = stripped.chars().collect();
    let n = chars.len();

    // Find leftmost top-level ○; restrict scan to before its function name.
    let mut call_start = n;
    {
        let mut paren = 0i64;
        let mut bracket = 0i64;
        for (i, &c) in chars.iter().enumerate() {
            match c {
                '(' => paren += 1,
                ')' => paren -= 1,
                '[' => bracket += 1,
                ']' => bracket -= 1,
                '○' if paren == 0 && bracket == 0 => {
                    let mut j: i64 = i as i64 - 1;
                    while j >= 0 && chars[j as usize].is_whitespace() {
                        j -= 1;
                    }
                    while j >= 0 && (chars[j as usize].is_alphanumeric() || chars[j as usize] == '_')
                    {
                        j -= 1;
                    }
                    call_start = (j + 1) as usize;
                    break;
                }
                _ => {}
            }
        }
    }

    for ops in BINOP_PRECEDENCE {
        let mut paren = 0i64;
        let mut bracket = 0i64;
        let mut i: i64 = call_start as i64 - 1;
        while i >= 0 {
            let c = chars[i as usize];
            match c {
                ')' => paren += 1,
                '(' => paren -= 1,
                ']' => bracket += 1,
                '[' => bracket -= 1,
                _ => {}
            }
            if paren == 0 && bracket == 0 {
                for op in *ops {
                    let l = op.chars().count() as i64;
                    let start = i - l + 1;
                    if start >= 0 {
                        let candidate: String =
                            chars[start as usize..=(i as usize)].iter().collect();
                        if candidate == *op {
                            let lhs: String = chars[..start as usize].iter().collect();
                            let rhs: String = chars[(i as usize + 1)..].iter().collect();
                            let lhs = lhs.trim().to_string();
                            let rhs = rhs.trim().to_string();
                            if !lhs.is_empty() && !rhs.is_empty() {
                                return Some((lhs, op.to_string(), rhs));
                            }
                        }
                    }
                }
            }
            i -= 1;
        }
    }
    None
}

/// Decode a UAGL string literal payload (the text between the quotes) into
/// raw bytes. Supports the C-style escapes commonly needed for UART/log
/// strings: `\n \r \t \\ \" \' \0 \xHH`. Other backslashes pass through
/// literally (the backslash byte itself, then the next character is
/// re-scanned normally on the following iteration).
pub fn decode_uagl_string(raw: &str) -> Vec<u8> {
    let chars: Vec<char> = raw.chars().collect();
    let n = chars.len();
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0usize;
    while i < n {
        let c = chars[i];
        if c == '\\' && i + 1 < n {
            let esc = chars[i + 1];
            match esc {
                'n' => {
                    out.push(0x0a);
                    i += 2;
                }
                'r' => {
                    out.push(0x0d);
                    i += 2;
                }
                't' => {
                    out.push(0x09);
                    i += 2;
                }
                '\\' => {
                    out.push(0x5c);
                    i += 2;
                }
                '"' => {
                    out.push(0x22);
                    i += 2;
                }
                '\'' => {
                    out.push(0x27);
                    i += 2;
                }
                '0' => {
                    out.push(0x00);
                    i += 2;
                }
                'x' if i + 3 < n => {
                    let hex: String = chars[i + 2..i + 4].iter().collect();
                    match u8::from_str_radix(&hex, 16) {
                        Ok(byte) => {
                            out.push(byte);
                            i += 4;
                        }
                        Err(_) => {
                            // Matches Python's `out.append(ord(c))` where
                            // `c` is still the backslash at this point.
                            out.push(b'\\');
                            i += 1;
                        }
                    }
                }
                _ => {
                    out.push(b'\\');
                    i += 1;
                }
            }
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            i += 1;
        }
    }
    out
}

/// Encode `op dst, src, #imm` legally for AArch64. AArch64 ADD/SUB take a
/// 12-bit unsigned immediate, optionally shifted left 12. Values that
/// don't fit are split into multiple instructions or routed through the
/// X9 scratch register. `imm` is assumed non-negative (every call site in
/// this codebase passes a stack offset or frame size).
pub fn aarch64_addsub_imm_lines(op: &str, dst: &str, src: &str, imm: i64, comment: &str) -> Vec<String> {
    if imm == 0 {
        if dst == src {
            return Vec::new();
        }
        return vec![format!("\tMOV\t{}, {}\t{} // imm=0 → MOV", dst, src, comment)];
    }
    let high = imm & !0xFFFi64;
    let low = imm & 0xFFF;
    if high == 0 {
        return vec![format!("\t{}\t{}, {}, #{}\t{}", op, dst, src, low, comment)];
    }
    if (high >> 12) <= 0xFFF {
        let mut lines = vec![format!(
            "\t{}\t{}, {}, #{}, LSL #12\t{}",
            op,
            dst,
            src,
            high >> 12,
            comment
        )];
        if low != 0 {
            lines.push(format!("\t{}\t{}, {}, #{}", op, dst, dst, low));
        }
        return lines;
    }
    let mut lines = vec![format!("\tMOVZ\tX9, #{}", imm & 0xFFFF)];
    if (imm >> 16) & 0xFFFF != 0 {
        lines.push(format!("\tMOVK\tX9, #{}, LSL #16", (imm >> 16) & 0xFFFF));
    }
    if (imm >> 32) & 0xFFFF != 0 {
        lines.push(format!("\tMOVK\tX9, #{}, LSL #32", (imm >> 32) & 0xFFFF));
    }
    lines.push(format!("\t{}\t{}, {}, X9\t{}", op, dst, src, comment));
    lines
}

/// `LDR dst, [X29, #off]` with a fallback for offsets beyond the 12-bit
/// scaled-immediate range (max ~32760 for X-register loads). Big offsets
/// are materialised via X16 (IP0, scratch per AArch64 ABI) and a 0-offset
/// load. Not currently called from the generator, but ported for parity
/// with the original public surface.
#[allow(dead_code)]
pub fn fp_ldr_lines(dst_reg: &str, off: i64, comment: &str) -> Vec<String> {
    if (0..=32760).contains(&off) && off % 8 == 0 {
        return vec![format!("\tLDR\t{}, [X29, #{}]\t{}", dst_reg, off, comment)];
    }
    let mut lines = aarch64_addsub_imm_lines(
        "ADD",
        "X16",
        "X29",
        off,
        &format!("// big-off addr ({})", off),
    );
    lines.push(format!("\tLDR\t{}, [X16]\t{}", dst_reg, comment));
    lines
}

/// `STR src, [X29, #off]` with the same big-offset fallback as
/// `fp_ldr_lines`. Not currently called from the generator, but ported for
/// parity with the original public surface.
#[allow(dead_code)]
pub fn fp_str_lines(src_reg: &str, off: i64, comment: &str) -> Vec<String> {
    if (0..=32760).contains(&off) && off % 8 == 0 {
        return vec![format!("\tSTR\t{}, [X29, #{}]\t{}", src_reg, off, comment)];
    }
    let mut lines = aarch64_addsub_imm_lines(
        "ADD",
        "X16",
        "X29",
        off,
        &format!("// big-off addr ({})", off),
    );
    lines.push(format!("\tSTR\t{}, [X16]\t{}", src_reg, comment));
    lines
}
