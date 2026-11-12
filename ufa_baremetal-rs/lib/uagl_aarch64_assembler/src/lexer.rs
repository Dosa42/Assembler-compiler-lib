//! Lexer — a faithful port of the regex table, `_collapse_multiline_array_globals`,
//! and `lex_uagl` from the Python original.
//!
//! One deliberate substitution: `_INLINE_COMMENT_RE` used a negative
//! lookahead (`(?:\*(?!/)[^*]*)*`) to skip past interior `*` characters
//! without prematurely closing on them. Rust's `regex` crate has no
//! lookaround (by design, to keep linear-time guarantees), but a
//! non-greedy `.*?` between the delimiters is a classic, exactly
//! equivalent rewrite for "shortest span up to the first closing
//! delimiter" — both pick the same match boundary. Every other pattern in
//! this file has no lookaround/backreferences and translates verbatim.

use crate::token::{Token, TokenData, TK};
use once_cell::sync::Lazy;
use regex::Regex;

fn re(pat: &str) -> Regex {
    Regex::new(pat).unwrap_or_else(|e| panic!("bad regex {pat:?}: {e}"))
}

static FUNC_DEF_RE: Lazy<Regex> = Lazy::new(|| re(r"^△\s+(\w+)\s*○\s*(.+?)\s*═╗\s*$"));
static VAR_DECL_RE: Lazy<Regex> =
    Lazy::new(|| re(r"^(?:║\s*)+·\s+(\w+)\s*─\s*([\w\[\]@]+)\s*─\s*(.+?)\s*◇\s*$"));
static ASSIGN_RE: Lazy<Regex> = Lazy::new(|| {
    re(r"^(?:║\s*)+(\w[\w.]*(?:\[[^\]]+\])?(?:\s*─\s*△\s*\w+(?:\[[^\]]+\])?)*)\s*─\s*(.+?)\s*◇\s*$")
});
static RETURN_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)*→\s+(.+?)\s*◇\s*$"));
static IF_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)*Φ\s+(.+?)\s*═╗\s*$"));
static ELSE_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)*▽\s*═╗\s*$"));
static LOOP_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)*ⁿ\s+(.+?)\s*═╗\s*$"));
static CALL_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)+(\w+)\s+○\s*(.*?)\s*◇\s*$"));
static BARRIER_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)*□\s*◇\s*$"));
static SCOPE_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)*○\s+(.+?)\s*◇\s*$"));
static VALIDATE_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)*Ω\s+○\s*(.*?)\s*◇\s*$"));
static NOP_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)*∅\s*◇\s*$"));
static BLOCK_CLOSE_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)*╚═══\s*(\d+)\s*═══╝\s*$"));
static MODULE_OPEN_RE: Lazy<Regex> = Lazy::new(|| re(r"^╔═\s*(\w+)\s*═╗\s*$"));
static STRUCT_OPEN_RE: Lazy<Regex> = Lazy::new(|| re(r"^□\s+(\w+)\s*═╗\s*$"));
static FIELD_RE: Lazy<Regex> = Lazy::new(|| re(r"^\s*║\s{1,2}(\w+)\s*─\s*([\w\[\]]+)\s*$"));
static PRAGMA_RE: Lazy<Regex> = Lazy::new(|| re(r"^#PRAGMA\s+(.+)$"));
static COMMENT_RE: Lazy<Regex> = Lazy::new(|| re(r"^\s*/\*.*\*/\s*$"));
static GLOBAL_RE: Lazy<Regex> =
    Lazy::new(|| re(r"^·\s+(\w+)\s*─\s*([\w\[\]]+)\s*─\s*(.+?)\s*◇\s*$"));
static TUPLE_RECV_RE: Lazy<Regex> =
    Lazy::new(|| re(r"^(?:║\s*)+←\s+(\w+)\s*─\s*(\w+)\s*─\s*(\w+)\s*○\s*(.*?)\s*◇\s*$"));

// See module doc: rewritten from a negative-lookahead pattern to an
// equivalent non-greedy one (Rust's `regex` crate has no lookaround).
static INLINE_COMMENT_RE: Lazy<Regex> = Lazy::new(|| re(r"\s*/\*.*?\*/\s*$"));

fn has_block_comment_open(line: &str) -> bool {
    line.contains("/*")
}
fn has_block_comment_close(line: &str) -> bool {
    line.contains("*/")
}

static ARRAY_GLOBAL_OPEN_RE: Lazy<Regex> =
    Lazy::new(|| re(r"^(·\s+\w+\s*─\s*\w+\[\d+\]\s*─\s*)\{\s*$"));
static ARRAY_GLOBAL_CLOSE_RE: Lazy<Regex> = Lazy::new(|| re(r"^(?:║\s*)*\}\s*(?:◇\s*)?$"));

/// Pre-pass: collapse multi-line array initializers into single lines.
///
/// Recognized open: line of form `· NAME ─ TYPE[N] ─ {` (brace at end).
/// Recognized close: any line whose stripped content is `}` or `} ◇`,
/// optionally prefixed with `║`.
/// Body lines: `║  v1, v2, v3,` — strip `║` and trailing comma.
///
/// Turns the multi-line block into: `· NAME ─ TYPE[N] ─ [v1,v2,v3] ◇`
/// so the existing GLOBAL_RE matches.
fn collapse_multiline_array_globals(src: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line_no = i + 1;
        let trimmed = lines[i].trim();
        let caps = ARRAY_GLOBAL_OPEN_RE.captures(trimmed);
        let Some(caps) = caps else {
            out.push((line_no, lines[i].to_string()));
            i += 1;
            continue;
        };
        let prefix = caps[1].to_string();
        let mut values: Vec<String> = Vec::new();
        i += 1;
        while i < lines.len() {
            let stripped = lines[i].trim();
            if ARRAY_GLOBAL_CLOSE_RE.is_match(stripped) || stripped.starts_with("╚═══") {
                break;
            }
            let mut l = stripped;
            if l.starts_with('║') {
                l = l.trim_start_matches('║').trim();
            }
            let l = l.trim_end_matches(',').trim();
            if !l.is_empty() {
                for tok in l.split(',') {
                    let tok = tok.trim();
                    if !tok.is_empty() {
                        values.push(tok.to_string());
                    }
                }
            }
            i += 1;
        }
        i += 1; // consume the closing line (or run off EOF if unterminated)
        out.push((line_no, format!("{}[{}] ◇", prefix, values.join(","))));
    }
    out
}

fn set_data(data: &mut TokenData, field: &str, value: String) {
    match field {
        "name" => data.name = Some(value),
        "params_raw" => data.params_raw = Some(value),
        "type" => data.r#type = Some(value),
        "value" => data.value = Some(value),
        "cond" => data.cond = Some(value),
        "func" => data.func = Some(value),
        "args" => data.args = Some(value),
        "text" => data.text = Some(value),
        "var1" => data.var1 = Some(value),
        "var2" => data.var2 = Some(value),
        "target" => data.target = Some(value),
        _ => unreachable!("unknown token data field {field}"),
    }
}

/// Tokenize UAGL v3 source into a flat token stream. Mirrors `lex_uagl`
/// exactly, including its if/elif precedence order (first matching rule
/// wins) and the block-comment state machine.
pub fn lex_uagl(src: &str) -> Vec<Token> {
    lex(src, false)
}

pub(crate) fn lex_for_validation(src: &str) -> Vec<Token> {
    lex(src, true)
}

fn lex(src: &str, report_unrecognized: bool) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut in_block_comment = false;

    for (index, (source_line, raw)) in collapse_multiline_array_globals(src).into_iter().enumerate() {
        let no = if report_unrecognized { source_line } else { index + 1 };
        let raw = raw.as_str();
        let line_owned = raw.trim();
        if line_owned.is_empty() {
            continue;
        }
        let mut line: String = line_owned.to_string();

        if in_block_comment {
            let mut t = Token::new(TK::Comment, raw, no);
            set_data(&mut t.data, "text", line.clone());
            tokens.push(t);
            if has_block_comment_close(&line) {
                in_block_comment = false;
            }
            continue;
        }
        if has_block_comment_open(&line) && !has_block_comment_close(&line) {
            let mut t = Token::new(TK::Comment, raw, no);
            set_data(&mut t.data, "text", line.clone());
            tokens.push(t);
            in_block_comment = true;
            continue;
        }

        if !(line.starts_with("/*") && line.ends_with("*/")) {
            line = INLINE_COMMENT_RE.replace(&line, "").to_string();
            if line.is_empty() {
                continue;
            }
        }

        if let Some(caps) = PRAGMA_RE.captures(&line) {
            let mut t = Token::new(TK::Pragma, raw, no);
            set_data(&mut t.data, "value", caps[1].to_string());
            tokens.push(t);
        } else if COMMENT_RE.is_match(&line) || line.starts_with("//") {
            let mut t = Token::new(TK::Comment, raw, no);
            set_data(&mut t.data, "text", line.clone());
            tokens.push(t);
        } else if let Some(caps) = MODULE_OPEN_RE.captures(&line) {
            let mut t = Token::new(TK::ModuleOpen, raw, no);
            set_data(&mut t.data, "name", caps[1].to_string());
            tokens.push(t);
        } else if let Some(caps) = STRUCT_OPEN_RE.captures(&line) {
            let mut t = Token::new(TK::StructDef, raw, no);
            set_data(&mut t.data, "name", caps[1].to_string());
            tokens.push(t);
        } else if let Some(caps) = FIELD_RE.captures(&line) {
            let mut t = Token::new(TK::Field, raw, no);
            set_data(&mut t.data, "name", caps[1].to_string());
            set_data(&mut t.data, "type", caps[2].to_string());
            tokens.push(t);
        } else if let Some(caps) = BLOCK_CLOSE_RE.captures(&line) {
            let mut t = Token::new(TK::BlockClose, raw, no);
            t.data.depth = caps[1].parse::<i64>().ok();
            tokens.push(t);
        } else if let Some(caps) = FUNC_DEF_RE.captures(&line) {
            let mut t = Token::new(TK::FuncDef, raw, no);
            set_data(&mut t.data, "name", caps[1].to_string());
            set_data(&mut t.data, "params_raw", caps[2].to_string());
            tokens.push(t);
        } else if let Some(caps) = VAR_DECL_RE.captures(&line) {
            let mut t = Token::new(TK::VarDecl, raw, no);
            set_data(&mut t.data, "name", caps[1].to_string());
            set_data(&mut t.data, "type", caps[2].to_string());
            set_data(&mut t.data, "value", caps[3].to_string());
            tokens.push(t);
        } else if let Some(caps) = GLOBAL_RE.captures(&line) {
            let mut t = Token::new(TK::Global, raw, no);
            set_data(&mut t.data, "name", caps[1].to_string());
            set_data(&mut t.data, "type", caps[2].to_string());
            set_data(&mut t.data, "value", caps[3].to_string());
            tokens.push(t);
        } else if let Some(caps) = TUPLE_RECV_RE.captures(&line) {
            let mut t = Token::new(TK::TupleRecv, raw, no);
            set_data(&mut t.data, "var1", caps[1].to_string());
            set_data(&mut t.data, "var2", caps[2].to_string());
            set_data(&mut t.data, "func", caps[3].to_string());
            set_data(&mut t.data, "args", caps[4].to_string());
            tokens.push(t);
        } else if let Some(caps) = RETURN_RE.captures(&line) {
            let mut t = Token::new(TK::Return, raw, no);
            set_data(&mut t.data, "value", caps[1].to_string());
            tokens.push(t);
        } else if let Some(caps) = IF_RE.captures(&line) {
            let mut t = Token::new(TK::If, raw, no);
            set_data(&mut t.data, "cond", caps[1].to_string());
            tokens.push(t);
        } else if ELSE_RE.is_match(&line) {
            tokens.push(Token::new(TK::Else, raw, no));
        } else if let Some(caps) = LOOP_RE.captures(&line) {
            let mut t = Token::new(TK::Loop, raw, no);
            set_data(&mut t.data, "cond", caps[1].to_string());
            tokens.push(t);
        } else if BARRIER_RE.is_match(&line) {
            tokens.push(Token::new(TK::Barrier, raw, no));
        } else if let Some(caps) = VALIDATE_RE.captures(&line) {
            let mut t = Token::new(TK::Validate, raw, no);
            set_data(&mut t.data, "args", caps[1].to_string());
            tokens.push(t);
        } else if let Some(caps) = SCOPE_RE.captures(&line) {
            let mut t = Token::new(TK::Scope, raw, no);
            set_data(&mut t.data, "args", caps[1].to_string());
            tokens.push(t);
        } else if NOP_RE.is_match(&line) {
            tokens.push(Token::new(TK::Nop, raw, no));
        } else if let Some(caps) = CALL_RE.captures(&line) {
            let mut t = Token::new(TK::Call, raw, no);
            set_data(&mut t.data, "func", caps[1].to_string());
            set_data(&mut t.data, "args", caps[2].to_string());
            tokens.push(t);
        } else if let Some(caps) = ASSIGN_RE.captures(&line) {
            let mut t = Token::new(TK::Assign, raw, no);
            set_data(&mut t.data, "target", caps[1].to_string());
            set_data(&mut t.data, "value", caps[2].to_string());
            tokens.push(t);
        } else {
            if report_unrecognized {
                let is_decoration = line
                    .chars()
                    .all(|ch| ch.is_whitespace() || ('\u{2500}'..='\u{257f}').contains(&ch));
                let is_indented_comment = line
                    .trim_start_matches(|ch: char| ch == '║' || ch.is_whitespace())
                    .starts_with("//");
                if !is_decoration && !is_indented_comment {
                    tokens.push(Token::new(TK::Unknown, raw, no));
                }
                continue;
            }
            let first_two: String = line.chars().take(2).collect();
            if !line.is_empty()
                && !line.starts_with('╔')
                && !line.starts_with('╚')
                && !first_two.contains('║')
            {
                tokens.push(Token::new(TK::Unknown, raw, no));
            }
        }
    }

    tokens
}
