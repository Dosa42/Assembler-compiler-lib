//! Tiny hand-rolled string-matching primitives used throughout this crate
//! in place of a regex engine.
//!
//! The Python original's brief (§5.1) mandates "stdlib only, no third-party
//! deps beyond pytest"; this port keeps that discipline by hand-writing
//! matchers for the fixed-grammar AArch64 assembly line shapes the parser
//! actually needs, instead of depending on the `regex` crate. Every
//! matcher built from these primitives (in `parse_s.rs` / `k_invariants.rs`)
//! is a direct translation of one specific Python `re` pattern, checked
//! against that pattern's actual match/no-match behavior on realistic
//! `.s` lines before being written — not a guess at what a regex "usually"
//! means.
//!
//! These are deliberately narrow: ASCII identifiers, decimal integers, and
//! literal/whitespace handling, which is all the patterns in this codebase
//! need. This is not a general regex engine and isn't meant to become one.

/// Skip leading whitespace, mirroring `\s*` at the start of a Python
/// pattern applied to a single (already newline-free) source line.
pub fn skip_ws(s: &str) -> &str {
    s.trim_start_matches(|c: char| c.is_whitespace())
}

/// Consume a case-sensitive literal after skipping leading whitespace.
/// Mirrors `\s*LITERAL` in a Python pattern — does NOT require anything
/// in particular to follow. Use [`lit_req_ws`] where the Python pattern
/// has `LITERAL\s+` (mandatory whitespace after the literal, typically
/// right before an identifier) — plain `lit` followed by `ident`/`digits`
/// would silently accept zero whitespace there, which is a real behavior
/// difference from the Python pattern, not just a style choice.
pub fn lit<'a>(s: &'a str, literal: &str) -> Option<&'a str> {
    skip_ws(s).strip_prefix(literal)
}

/// Consume a literal that MUST be followed by at least one whitespace
/// character (or end of the already-trimmed line), mirroring `LITERAL\s+`
/// in a Python pattern. The returned remainder still has that whitespace
/// in it — callers compose with `skip_ws`/`ident`/etc. as usual.
pub fn lit_req_ws<'a>(s: &'a str, literal: &str) -> Option<&'a str> {
    let rest = skip_ws(s).strip_prefix(literal)?;
    if rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace()) {
        Some(rest)
    } else {
        None
    }
}

/// Case-insensitive version of [`lit`] — mirrors a literal matched under
/// Python's `re.IGNORECASE` flag (used by K05's barrier-mnemonic
/// patterns, e.g. `_MSR_DAIFSET_RE`).
pub fn lit_ci<'a>(s: &'a str, literal: &str) -> Option<&'a str> {
    let s = skip_ws(s);
    let head = s.get(..literal.len())?;
    if head.eq_ignore_ascii_case(literal) {
        Some(&s[literal.len()..])
    } else {
        None
    }
}

/// Case-insensitive version of [`lit_req_ws`].
pub fn lit_req_ws_ci<'a>(s: &'a str, literal: &str) -> Option<&'a str> {
    let rest = lit_ci(s, literal)?;
    if rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace()) {
        Some(rest)
    } else {
        None
    }
}

/// Consume an identifier matching `[A-Za-z_]\w*` (after skipping leading
/// whitespace). Returns `(identifier, rest)`.
pub fn ident(s: &str) -> Option<(&str, &str)> {
    let s = skip_ws(s);
    let mut chars = s.char_indices();
    let (_, c0) = chars.next()?;
    if !(c0.is_ascii_alphabetic() || c0 == '_') {
        return None;
    }
    let mut end = c0.len_utf8();
    for (i, c) in chars {
        if c.is_ascii_alphanumeric() || c == '_' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    Some((&s[..end], &s[end..]))
}

/// Matches a register-style token: a fixed one-char `prefix` followed by
/// `\w+` (one or more word characters), e.g. `(X\w+)` / `(W\w+)` in the
/// Python patterns. The prefix is part of the returned token (Python's
/// capture group includes it too — `(X\w+)` matched against `"X9"`
/// captures `"X9"`, not `"9"`).
///
/// Note: patterns like `(W\w+|WZR)` in the Python source are redundant in
/// practice — `WZR` already matches `W\w+` (`Z` and `R` are both word
/// characters), so this single helper covers both alternatives.
pub fn reg_token(s: &str, prefix: char) -> Option<(String, &str)> {
    let s = skip_ws(s);
    let rest = s.strip_prefix(prefix)?;
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    if end == 0 {
        return None; // `\w+` requires at least one char after the prefix
    }
    let mut token = String::with_capacity(1 + end);
    token.push(prefix);
    token.push_str(&rest[..end]);
    Some((token, &rest[end..]))
}

/// Consume an unsigned decimal integer matching `\d+`.
pub fn digits(s: &str) -> Option<(&str, &str)> {
    let s = skip_ws(s);
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        None
    } else {
        Some((&s[..end], &s[end..]))
    }
}

/// Consume an optionally-signed decimal integer matching `-?\d+`.
pub fn signed_digits(s: &str) -> Option<(&str, &str)> {
    let s0 = skip_ws(s);
    let body = s0.strip_prefix('-').unwrap_or(s0);
    let digit_len = body.find(|c: char| !c.is_ascii_digit()).unwrap_or(body.len());
    if digit_len == 0 {
        return None;
    }
    let total = (s0.len() - body.len()) + digit_len;
    Some((&s0[..total], &s0[total..]))
}

/// True if, after skipping whitespace, nothing remains — mirrors `\s*$`
/// at the end of a Python pattern matched against a single line.
pub fn at_end(s: &str) -> bool {
    skip_ws(s).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ident_basic() {
        assert_eq!(ident("foo_bar123 rest"), Some(("foo_bar123", " rest")));
        assert_eq!(ident("  _leading rest"), Some(("_leading", " rest")));
        assert_eq!(ident("9nope"), None);
        assert_eq!(ident(""), None);
    }

    #[test]
    fn reg_token_covers_zr_alternative() {
        assert_eq!(reg_token("X9, rest", 'X'), Some(("X9".to_string(), ", rest")));
        assert_eq!(reg_token("X29]", 'X'), Some(("X29".to_string(), "]")));
        assert_eq!(reg_token("XZR, rest", 'X'), Some(("XZR".to_string(), ", rest")));
        assert_eq!(reg_token("WZR]", 'W'), Some(("WZR".to_string(), "]")));
        assert_eq!(reg_token("Y9", 'X'), None);
        assert_eq!(reg_token("X", 'X'), None); // \w+ needs >=1 char after prefix
    }

    #[test]
    fn digits_and_signed_digits() {
        assert_eq!(digits("48, rest"), Some(("48", ", rest")));
        assert_eq!(digits("abc"), None);
        assert_eq!(signed_digits("-16]"), Some(("-16", "]")));
        assert_eq!(signed_digits("16]"), Some(("16", "]")));
        assert_eq!(signed_digits("-"), None);
    }

    #[test]
    fn lit_req_ws_rejects_zero_whitespace() {
        // ".type" immediately followed by an identifier with no space
        // between them must NOT match \.type\s+ -- this is the exact bug
        // a naive `lit()` + `ident()` composition would introduce.
        assert_eq!(lit_req_ws(".typefoo, %function", ".type"), None);
        assert_eq!(lit_req_ws(".type foo, %function", ".type").map(skip_ws), Some("foo, %function"));
    }

    #[test]
    fn at_end_basic() {
        assert!(at_end("   "));
        assert!(at_end(""));
        assert!(!at_end("  x"));
    }
}
