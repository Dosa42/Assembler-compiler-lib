//! Port of `parse_s.py`.
//!
//! Structural parsing of emitted `.s` text: line splitting (code vs.
//! trailing `//` comment), function-body extraction, prologue/epilogue and
//! scratch-spill classification, frame-pointer-relative access
//! classification, data-global extraction, and signed-branch/compare site
//! detection. Every K-check in [`crate::k_invariants`] is built on top of
//! these primitives.
//!
//! This module hand-rolls line-shape matching instead of depending on the
//! `regex` crate (see [`crate::pattern`] for why and how); each matcher
//! function's doc comment names the exact Python `re` pattern it replaces.

use std::collections::HashMap;

use crate::pattern;

// ---------------------------------------------------------------------
// Line
// ---------------------------------------------------------------------

/// One physical line of `.s` text, already split into its code and
/// (stripped, `//`-marker-removed) trailing-comment parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub no: u32,
    pub raw: String,
    pub code: String,
    pub comment: String,
}

impl Line {
    pub fn is_blank(&self) -> bool {
        self.code.is_empty()
    }

    /// A bare `label:` with nothing else on the line.
    pub fn is_label(&self) -> bool {
        !self.code.is_empty() && self.code.ends_with(':') && !self.code.contains(' ')
    }
}

/// Mirrors `_COMMENT_RE = re.compile(r"//.*$")` used via `.search()`: finds
/// the FIRST `//` anywhere in the line and treats everything from there to
/// end-of-line as the comment. This is naive about quoted strings — a
/// `.asciz "http://x"` line truncates at the `//` inside the URL in BOTH
/// the Python original and this port. Faithfully preserved, not a
/// Rust-specific bug.
fn split_code_comment(raw: &str) -> (String, String) {
    match raw.find("//") {
        Some(idx) => {
            let code = raw[..idx].trim().to_string();
            let comment = raw[idx..].trim_start_matches('/').trim().to_string();
            (code, comment)
        }
        None => (raw.trim().to_string(), String::new()),
    }
}

/// Splits on `"\n"`, trimming a trailing `"\r"` from each piece so
/// `"\r\n"` behaves like `"\n"`, and — matching Python's `splitlines()` —
/// a final trailing newline does not produce a trailing empty line, and
/// an empty string produces zero lines. This covers the line-ending
/// conventions that actually occur in emitted/hand-written `.s` text;
/// Python's `splitlines()` additionally recognizes a handful of rarer
/// Unicode line boundaries (bare `\r`, `\v`, `\f`, `\x1c`-`\x1e`, `\x85`,
/// U+2028, U+2029) that don't occur in ASCII assembly source and aren't
/// exercised by anything in this package.
fn normalize_lines(s_text: &str) -> Vec<String> {
    if s_text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = s_text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
        .collect();
    if s_text.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// Mirrors `split_lines()` in `parse_s.py`.
pub fn split_lines(s_text: &str) -> Vec<Line> {
    normalize_lines(s_text)
        .into_iter()
        .enumerate()
        .map(|(i, raw)| {
            let (code, comment) = split_code_comment(&raw);
            Line { no: (i + 1) as u32, raw, code, comment }
        })
        .collect()
}

// ---------------------------------------------------------------------
// Function + parse_functions
// ---------------------------------------------------------------------

/// A parsed function body: everything between its `NAME:` label and its
/// end marker (a matching `.size NAME, . - NAME`, the line before a
/// different function's `.type` declaration, or end-of-file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub body: Vec<Line>,
    pub params: Vec<(String, String)>,
    pub ret_type: String,
}

impl Function {
    pub fn line_range(&self) -> (u32, u32) {
        (self.start_line, self.end_line)
    }
}

/// `^\.type\s+([A-Za-z_]\w*)\s*,\s*%function\s*$`
fn match_type_function(code: &str) -> Option<String> {
    let rest = pattern::lit_req_ws(code, ".type")?;
    let (name, rest) = pattern::ident(rest)?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "%function")?;
    if pattern::at_end(rest) {
        Some(name.to_string())
    } else {
        None
    }
}

/// `^([A-Za-z_]\w*):\s*$` (shared by both `_LABEL_RE` in `parse_functions`
/// and `_LABEL_DECL_RE` in `parse_globals` — byte-identical patterns).
fn match_label(code: &str) -> Option<String> {
    let (name, rest) = pattern::ident(code)?;
    let rest = pattern::lit(rest, ":")?;
    if pattern::at_end(rest) {
        Some(name.to_string())
    } else {
        None
    }
}

/// `^\.size\s+([A-Za-z_]\w*)\s*,\s*\.\s*-\s*([A-Za-z_]\w*)\s*$`
fn match_size(code: &str) -> Option<(String, String)> {
    let rest = pattern::lit_req_ws(code, ".size")?;
    let (a, rest) = pattern::ident(rest)?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, ".")?;
    let rest = pattern::lit(rest, "-")?;
    let (b, rest) = pattern::ident(rest)?;
    if pattern::at_end(rest) {
        Some((a.to_string(), b.to_string()))
    } else {
        None
    }
}

/// `params:\s*\[(.*?)\]\s+ret:\s*([A-Za-z_]\w*)`, used via `.search()` in
/// the Python original (unanchored — "params:" may occur anywhere in the
/// comment text).
fn match_params_comment(comment: &str) -> Option<(String, String)> {
    let idx = comment.find("params:")?;
    let rest = &comment[idx + "params:".len()..];
    let rest = pattern::skip_ws(rest);
    let rest = rest.strip_prefix('[')?;
    // Non-greedy `(.*?)\]`: in every real "// params: [...] ret: TYPE"
    // comment the assembler emits, the parameter-pair list never itself
    // contains a literal ']' (see `_PARAM_PAIR_RE`'s type charset below,
    // which excludes brackets) — so "first ']' from here" and "shortest
    // match ending in ']'" agree in practice.
    let close = rest.find(']')?;
    let inner = &rest[..close];
    let after = &rest[close + 1..];
    if !after.starts_with(|c: char| c.is_whitespace()) {
        return None; // `\s+` before "ret:" is mandatory
    }
    let after = pattern::skip_ws(after);
    let after = after.strip_prefix("ret:")?;
    let (ret_type, _rest) = pattern::ident(after)?;
    Some((inner.to_string(), ret_type.to_string()))
}

/// `\(\s*'([^']+)'\s*,\s*'([A-Za-z_@][\w@]*)'\s*\)`, used via `.findall()`
/// — scans for ALL non-overlapping matches in `inner`.
fn find_param_pairs(inner: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = inner[pos..].find('(') {
        let start = pos + rel;
        if let Some((pname, ptype, matched_len)) = try_match_param_pair(&inner[start..]) {
            out.push((pname, ptype));
            pos = start + matched_len;
        } else {
            pos = start + 1;
        }
    }
    out
}

fn try_match_param_pair(s: &str) -> Option<(String, String, usize)> {
    let rest = s.strip_prefix('(')?;
    let rest = pattern::skip_ws(rest);
    let rest = rest.strip_prefix('\'')?;
    let close1 = rest.find('\'')?;
    if close1 == 0 {
        return None; // `[^']+` requires at least one char
    }
    let name = &rest[..close1];
    let rest = &rest[close1 + 1..];
    let rest = pattern::skip_ws(rest);
    let rest = rest.strip_prefix(',')?;
    let rest = pattern::skip_ws(rest);
    let rest = rest.strip_prefix('\'')?;
    // Type charset: `[A-Za-z_@][\w@]*` (no brackets/digits-first).
    let mut chars = rest.char_indices();
    let (_, c0) = chars.next()?;
    if !(c0.is_ascii_alphabetic() || c0 == '_' || c0 == '@') {
        return None;
    }
    let mut end = c0.len_utf8();
    for (i, c) in chars {
        if c.is_ascii_alphanumeric() || c == '_' || c == '@' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    let ptype = &rest[..end];
    let rest = &rest[end..];
    let rest = rest.strip_prefix('\'')?;
    let rest = pattern::skip_ws(rest);
    let rest = rest.strip_prefix(')')?;
    let total_len = s.len() - rest.len();
    Some((name.to_string(), ptype.to_string(), total_len))
}

/// Returns `lines[start..end]` as an owned `Vec`, or an empty `Vec` if
/// `start >= end` — Python slicing tolerates a start past the end
/// (`lst[5:2] == []`); Rust range-indexing would panic, so this guards it.
fn body_slice(lines: &[Line], start: usize, end: usize) -> Vec<Line> {
    if start >= end {
        Vec::new()
    } else {
        lines[start..end].to_vec()
    }
}

/// Mirrors `parse_functions()` in `parse_s.py`.
///
/// Note a faithfully-preserved quirk of the original: when a function's
/// end is determined by encountering the NEXT function's `.type` line
/// (rather than by its own `.size`), the line immediately before that
/// next `.type` becomes `end_line` — but is EXCLUDED from `body` (the
/// slice upper bound is exclusive). The same thing happens when a
/// function runs off the end of the file with no `.size` at all: the
/// very last line of the file is reported as `end_line` but is not part
/// of `body`. Empirically confirmed against the Python original before
/// porting (see `tests/golden.rs`); not something this port silently
/// "fixes".
pub fn parse_functions(s_text: &str) -> Vec<Function> {
    let lines = split_lines(s_text);
    let n = lines.len();
    let mut functions = Vec::new();

    let mut i = 0usize;
    while i < n {
        let name = match match_type_function(&lines[i].code) {
            Some(name) => name,
            None => {
                i += 1;
                continue;
            }
        };

        let mut label_idx: Option<usize> = None;
        for j in (i + 1)..n.min(i + 4) {
            if match_label(&lines[j].code).as_deref() == Some(name.as_str()) {
                label_idx = Some(j);
                break;
            }
        }
        let label_idx = match label_idx {
            Some(idx) => idx,
            None => {
                i += 1;
                continue;
            }
        };

        let mut end_idx: Option<usize> = None;
        for j in (label_idx + 1)..n {
            if let Some((a, b)) = match_size(&lines[j].code) {
                if a == name && b == name {
                    end_idx = Some(j);
                    break;
                }
            }
            if let Some(other) = match_type_function(&lines[j].code) {
                if other != name {
                    end_idx = Some(j - 1);
                    break;
                }
            }
        }
        let end_idx = end_idx.unwrap_or(n - 1);

        let body = body_slice(&lines, label_idx + 1, end_idx);

        let mut params = Vec::new();
        let mut ret_type = String::new();
        for cand in body.iter().take(6) {
            if let Some((inner, ret)) = match_params_comment(&cand.comment) {
                ret_type = ret;
                params = find_param_pairs(&inner);
                break;
            }
        }

        functions.push(Function {
            name,
            start_line: lines[label_idx].no,
            end_line: lines[end_idx].no,
            body,
            params,
            ret_type,
        });

        i = end_idx + 1;
    }

    functions
}

// ---------------------------------------------------------------------
// ParseCompletenessError + assert_parse_complete
// ---------------------------------------------------------------------

/// Raised (as an `Err`) when `parse_functions` would silently drop a
/// `.type NAME, %function` declaration that never found its label within
/// the 3-line lookahead window. Per Council audit 2026-05-06: a K-check
/// pass over a partial parse would be vacuous PASSes masking real bugs,
/// so `codegen_audit::audit_file` refuses to run any K-check at all when
/// this fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseCompletenessError(pub String);

impl std::fmt::Display for ParseCompletenessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseCompletenessError {}

/// Mirrors `assert_parse_complete()` in `parse_s.py`.
pub fn assert_parse_complete(s_text: &str, filename: &str) -> Result<(), ParseCompletenessError> {
    let lines = split_lines(s_text);
    let mut declared: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for ln in &lines {
        if let Some(name) = match_type_function(&ln.code) {
            declared.entry(name).or_insert(ln.no);
        }
    }

    let parsed: std::collections::HashSet<String> =
        parse_functions(s_text).into_iter().map(|f| f.name).collect();

    // BTreeMap keys already iterate in sorted order, matching Python's
    // `sorted(set(declared) - parsed)`.
    let missing: Vec<String> = declared
        .keys()
        .filter(|k| !parsed.contains(k.as_str()))
        .cloned()
        .collect();

    if let Some(first) = missing.first() {
        let first_line = declared[first];
        let quoted: Vec<String> = missing.iter().map(|s| format!("'{s}'")).collect();
        return Err(ParseCompletenessError(format!(
            "{filename}: parse_functions silently dropped {} function-typed declaration(s): [{}]. \
First offender: `.type {first}, %function` at {filename}:{first_line}. \
Refusing to emit K-check verdicts on a partial parse.",
            missing.len(),
            quoted.join(", "),
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------
// SpAdjust: SUB/ADD SP, STP/LDP frame prologue/epilogue, scratch spills
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpAdjustKind {
    SubSp,
    AddSp,
    StpPrologue,
    LdpEpilogue,
    SpillPush,
    SpillPop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpAdjust {
    pub line_no: u32,
    pub kind: SpAdjustKind,
    pub amount: u64,
    /// Register touched, for the two spill kinds (empty for the other
    /// four, matching the Python dataclass's `reg: str = ""` default —
    /// STP/LDP frame prologue/epilogue instead store the fixed
    /// `"X29,X30"` marker, exactly as the Python does).
    pub reg: String,
}

/// `,\s*#(-?\d+)` — the optional offset suffix shared by every
/// frame/stack-relative access pattern below (`[X29(, #N)?]` etc.).
/// Returns `(offset, remainder_after_it)`. `pub(crate)`: `k_invariants.rs`
/// reuses this for `_LDR_W_FROM_FP_RE`.
pub(crate) fn try_offset_suffix(s: &str) -> Option<(i64, &str)> {
    let after_comma = pattern::skip_ws(s).strip_prefix(',')?;
    let after_hash = pattern::lit(after_comma, "#")?;
    let (num, rest) = pattern::signed_digits(after_hash)?;
    let offset: i64 = num.parse().ok()?;
    Some((offset, rest))
}

/// `^(SUB|ADD)\s+SP\s*,\s*SP\s*,\s*#(\d+)\s*(?:,\s*LSL\s+#(\d+))?\s*$`
fn match_sub_add_sp(code: &str) -> Option<(bool, u64, Option<u32>)> {
    let (is_sub, rest) = if let Some(r) = pattern::lit_req_ws(code, "SUB") {
        (true, r)
    } else if let Some(r) = pattern::lit_req_ws(code, "ADD") {
        (false, r)
    } else {
        return None;
    };
    let rest = pattern::lit(rest, "SP")?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "SP")?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "#")?;
    let (n_str, after_n) = pattern::digits(rest)?;
    let n: u64 = n_str.parse().ok()?;

    // Optional `,\s*LSL\s+#(\d+)` — try it; on any structural failure,
    // fall back to treating it as absent (Python's `(?:...)?`
    // backtracking semantics).
    if let Some(after_comma) = pattern::skip_ws(after_n).strip_prefix(',') {
        if let Some(after_lsl) = pattern::lit_req_ws(after_comma, "LSL") {
            if let Some(after_hash) = pattern::lit(after_lsl, "#") {
                if let Some((s_str, after_shift)) = pattern::digits(after_hash) {
                    if pattern::at_end(after_shift) {
                        if let Ok(shift) = s_str.parse::<u32>() {
                            return Some((is_sub, n, Some(shift)));
                        }
                    }
                }
            }
        }
    }
    if pattern::at_end(after_n) {
        Some((is_sub, n, None))
    } else {
        None
    }
}

/// `^STP\s+X29\s*,\s*X30\s*,\s*\[\s*SP\s*,\s*#-16\s*\]\s*!\s*$`
fn matches_stp_prologue(code: &str) -> bool {
    (|| -> Option<()> {
        let rest = pattern::lit_req_ws(code, "STP")?;
        let rest = pattern::lit(rest, "X29")?;
        let rest = pattern::lit(rest, ",")?;
        let rest = pattern::lit(rest, "X30")?;
        let rest = pattern::lit(rest, ",")?;
        let rest = pattern::lit(rest, "[")?;
        let rest = pattern::lit(rest, "SP")?;
        let rest = pattern::lit(rest, ",")?;
        let rest = pattern::lit(rest, "#-16")?;
        let rest = pattern::lit(rest, "]")?;
        let rest = pattern::lit(rest, "!")?;
        pattern::at_end(rest).then_some(())
    })()
    .is_some()
}

/// `^LDP\s+X29\s*,\s*X30\s*,\s*\[\s*SP\s*\]\s*,\s*#16\s*$`
fn matches_ldp_epilogue(code: &str) -> bool {
    (|| -> Option<()> {
        let rest = pattern::lit_req_ws(code, "LDP")?;
        let rest = pattern::lit(rest, "X29")?;
        let rest = pattern::lit(rest, ",")?;
        let rest = pattern::lit(rest, "X30")?;
        let rest = pattern::lit(rest, ",")?;
        let rest = pattern::lit(rest, "[")?;
        let rest = pattern::lit(rest, "SP")?;
        let rest = pattern::lit(rest, "]")?;
        let rest = pattern::lit(rest, ",")?;
        let rest = pattern::lit(rest, "#16")?;
        pattern::at_end(rest).then_some(())
    })()
    .is_some()
}

/// `^STR\s+(X\w+)\s*,\s*\[\s*SP\s*,\s*#-16\s*\]\s*!\s*$`
fn match_spill_push(code: &str) -> Option<String> {
    let rest = pattern::lit_req_ws(code, "STR")?;
    let (reg, rest) = pattern::reg_token(rest, 'X')?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "[")?;
    let rest = pattern::lit(rest, "SP")?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "#-16")?;
    let rest = pattern::lit(rest, "]")?;
    let rest = pattern::lit(rest, "!")?;
    if pattern::at_end(rest) {
        Some(reg.to_uppercase())
    } else {
        None
    }
}

/// `^LDR\s+(X\w+)\s*,\s*\[\s*SP\s*\]\s*,\s*#16\s*$`
fn match_spill_pop(code: &str) -> Option<String> {
    let rest = pattern::lit_req_ws(code, "LDR")?;
    let (reg, rest) = pattern::reg_token(rest, 'X')?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "[")?;
    let rest = pattern::lit(rest, "SP")?;
    let rest = pattern::lit(rest, "]")?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "#16")?;
    if pattern::at_end(rest) {
        Some(reg.to_uppercase())
    } else {
        None
    }
}

/// Mirrors `classify_sp_adjust()` in `parse_s.py`.
pub fn classify_sp_adjust(line: &Line) -> Option<SpAdjust> {
    let code = &line.code;
    if code.is_empty() {
        return None;
    }
    if let Some((is_sub, n, lsl)) = match_sub_add_sp(code) {
        let amount = match lsl {
            Some(shift) => n << shift,
            None => n,
        };
        let kind = if is_sub { SpAdjustKind::SubSp } else { SpAdjustKind::AddSp };
        return Some(SpAdjust { line_no: line.no, kind, amount, reg: String::new() });
    }
    if matches_stp_prologue(code) {
        return Some(SpAdjust {
            line_no: line.no,
            kind: SpAdjustKind::StpPrologue,
            amount: 16,
            reg: "X29,X30".to_string(),
        });
    }
    if matches_ldp_epilogue(code) {
        return Some(SpAdjust {
            line_no: line.no,
            kind: SpAdjustKind::LdpEpilogue,
            amount: 16,
            reg: "X29,X30".to_string(),
        });
    }
    if let Some(reg) = match_spill_push(code) {
        if reg == "X29" || reg == "X30" {
            return None;
        }
        return Some(SpAdjust { line_no: line.no, kind: SpAdjustKind::SpillPush, amount: 16, reg });
    }
    if let Some(reg) = match_spill_pop(code) {
        if reg == "X29" || reg == "X30" {
            return None;
        }
        return Some(SpAdjust { line_no: line.no, kind: SpAdjustKind::SpillPop, amount: 16, reg });
    }
    None
}

/// Mirrors `collect_sp_adjusts()` in `parse_s.py`.
pub fn collect_sp_adjusts(fn_: &Function) -> Vec<SpAdjust> {
    fn_.body.iter().filter_map(classify_sp_adjust).collect()
}

// ---------------------------------------------------------------------
// FpAccess: frame-pointer-relative loads/stores
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpAccessOp {
    StrXzr,
    StrX,
    StrW,
    Strb,
    Stp,
    LdrX,
    Ldrsw,
    LdrW,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FpAccess {
    pub line_no: u32,
    pub op: FpAccessOp,
    pub reg: String,
    /// Second register, STP only (empty otherwise — mirrors the Python
    /// dataclass's `reg2: str = ""` default).
    pub reg2: String,
    pub offset: i64,
    pub comment: String,
}

/// The recurring `MNEMONIC\s+(PREFIX\w+)\s*,\s*\[\s*BASE\s*(?:,\s*#(-?\d+))?\s*\]\s*$`
/// shape shared by STR_X/STR_W/STRB/LDR_X/LDRSW/LDR_W.
fn match_reg_base_offset(code: &str, mnemonic: &str, reg_prefix: char, base: &str) -> Option<(String, i64)> {
    let rest = pattern::lit_req_ws(code, mnemonic)?;
    let (reg, rest) = pattern::reg_token(rest, reg_prefix)?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "[")?;
    let rest = pattern::lit(rest, base)?;
    let (offset, after_offset) = try_offset_suffix(rest).unwrap_or((0, rest));
    let rest = pattern::lit(after_offset, "]")?;
    if pattern::at_end(rest) {
        Some((reg.to_uppercase(), offset))
    } else {
        None
    }
}

/// Same shape as [`match_reg_base_offset`] but with a fixed (uncaptured)
/// register name — used for `STR XZR, [X29...]` and, from
/// `k_invariants.rs`, `STR X0, [X29...]` / `STR X1, [X29...]`.
pub(crate) fn match_fixed_reg_base_offset(code: &str, mnemonic: &str, fixed_reg: &str, base: &str) -> Option<i64> {
    let rest = pattern::lit_req_ws(code, mnemonic)?;
    let rest = pattern::lit(rest, fixed_reg)?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "[")?;
    let rest = pattern::lit(rest, base)?;
    let (offset, after_offset) = try_offset_suffix(rest).unwrap_or((0, rest));
    let rest = pattern::lit(after_offset, "]")?;
    if pattern::at_end(rest) {
        Some(offset)
    } else {
        None
    }
}

/// `^STP\s+(X\w+|XZR)\s*,\s*(X\w+|XZR)\s*,\s*\[\s*X29\s*(?:,\s*#(-?\d+))?\s*\]\s*$`
fn match_stp_fp(code: &str) -> Option<(String, String, i64)> {
    let rest = pattern::lit_req_ws(code, "STP")?;
    let (reg1, rest) = pattern::reg_token(rest, 'X')?;
    let rest = pattern::lit(rest, ",")?;
    let (reg2, rest) = pattern::reg_token(rest, 'X')?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "[")?;
    let rest = pattern::lit(rest, "X29")?;
    let (offset, after_offset) = try_offset_suffix(rest).unwrap_or((0, rest));
    let rest = pattern::lit(after_offset, "]")?;
    if pattern::at_end(rest) {
        Some((reg1.to_uppercase(), reg2.to_uppercase(), offset))
    } else {
        None
    }
}

/// Mirrors `classify_fp_access()` in `parse_s.py`. Check order matters:
/// `STR XZR` is tried before the general `STR (X\w+)` case, because `XZR`
/// also structurally matches `X\w+` (both `Z` and `R` are word
/// characters) — checking XZR first is what makes it win.
pub fn classify_fp_access(line: &Line) -> Option<FpAccess> {
    let code = &line.code;
    if code.is_empty() {
        return None;
    }
    let mk = |op: FpAccessOp, reg: String, reg2: String, offset: i64| -> FpAccess {
        FpAccess { line_no: line.no, op, reg, reg2, offset, comment: line.comment.clone() }
    };

    if let Some(off) = match_fixed_reg_base_offset(code, "STR", "XZR", "X29") {
        return Some(mk(FpAccessOp::StrXzr, "XZR".to_string(), String::new(), off));
    }
    if let Some((reg, off)) = match_reg_base_offset(code, "STR", 'X', "X29") {
        return Some(mk(FpAccessOp::StrX, reg, String::new(), off));
    }
    if let Some((reg, off)) = match_reg_base_offset(code, "STR", 'W', "X29") {
        return Some(mk(FpAccessOp::StrW, reg, String::new(), off));
    }
    if let Some((reg, off)) = match_reg_base_offset(code, "STRB", 'W', "X29") {
        return Some(mk(FpAccessOp::Strb, reg, String::new(), off));
    }
    if let Some((reg1, reg2, off)) = match_stp_fp(code) {
        return Some(mk(FpAccessOp::Stp, reg1, reg2, off));
    }
    if let Some((reg, off)) = match_reg_base_offset(code, "LDR", 'X', "X29") {
        return Some(mk(FpAccessOp::LdrX, reg, String::new(), off));
    }
    if let Some((reg, off)) = match_reg_base_offset(code, "LDRSW", 'X', "X29") {
        return Some(mk(FpAccessOp::Ldrsw, reg, String::new(), off));
    }
    if let Some((reg, off)) = match_reg_base_offset(code, "LDR", 'W', "X29") {
        return Some(mk(FpAccessOp::LdrW, reg, String::new(), off));
    }
    None
}

/// Mirrors `collect_fp_accesses()` in `parse_s.py`.
pub fn collect_fp_accesses(fn_: &Function) -> Vec<FpAccess> {
    fn_.body.iter().filter_map(classify_fp_access).collect()
}

// ---------------------------------------------------------------------
// build_offset_type_map
// ---------------------------------------------------------------------

/// `let\s+([A-Za-z_]\w*)\s*:\s*([A-Za-z_@][\w@\[\]\d]*)`, used via
/// `.search()`. Shared verbatim between `_LET_RE` (`parse_s.py`) and
/// `_LET_AT_K03_RE` (`k_invariants.py`) — the two Python constants compile
/// the identical pattern string, so this one function backs both.
pub fn find_let_annotation(text: &str) -> Option<(String, String)> {
    let mut search_from = 0usize;
    while let Some(rel) = text[search_from..].find("let") {
        let start = search_from + rel;
        let after_let = &text[start + 3..];
        if after_let.starts_with(|c: char| c.is_whitespace()) {
            if let Some(result) = try_match_let_rest(after_let) {
                return Some(result);
            }
        }
        search_from = start + 1;
    }
    None
}

fn try_match_let_rest(after_let: &str) -> Option<(String, String)> {
    let (name, rest) = pattern::ident(after_let)?;
    let rest = pattern::skip_ws(rest);
    let rest = rest.strip_prefix(':')?;
    let rest = pattern::skip_ws(rest);
    let mut chars = rest.char_indices();
    let (_, c0) = chars.next()?;
    if !(c0.is_ascii_alphabetic() || c0 == '_' || c0 == '@') {
        return None;
    }
    let mut end = c0.len_utf8();
    for (i, c) in chars {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '[' | ']') {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    Some((name.to_string(), rest[..end].to_string()))
}

/// `spill\s+param\s+([A-Za-z_]\w*)`, used via `.search()`.
fn match_spill_param(text: &str) -> Option<String> {
    let mut search_from = 0usize;
    while let Some(rel) = text[search_from..].find("spill") {
        let start = search_from + rel;
        let after = &text[start + "spill".len()..];
        if !after.starts_with(|c: char| c.is_whitespace()) {
            search_from = start + 1;
            continue;
        }
        let after = pattern::skip_ws(after);
        if let Some(after_param) = after.strip_prefix("param") {
            if after_param.starts_with(|c: char| c.is_whitespace()) {
                let after_param = pattern::skip_ws(after_param);
                if let Some((name, _rest)) = pattern::ident(after_param) {
                    return Some(name.to_string());
                }
            }
        }
        search_from = start + 1;
    }
    None
}

/// Mirrors `build_offset_type_map()` in `parse_s.py`: walks a function
/// body accumulating a `frame offset -> declared type` map from three
/// signal sources in the comments the assembler emits (`spill param X`,
/// `signed-load`, and `let x: TYPE`/pending-let-then-spill pairs).
pub fn build_offset_type_map(fn_: &Function) -> HashMap<i64, String> {
    let param_type: HashMap<&str, &str> =
        fn_.params.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    let mut off_type: HashMap<i64, String> = HashMap::new();
    let mut pending_let: Option<(String, String)> = None;

    for ln in &fn_.body {
        if ln.comment.contains("spill param") {
            let m_sp = match_spill_param(&ln.comment);
            let acc = classify_fp_access(ln);
            if let (Some(pname), Some(acc)) = (m_sp, &acc) {
                if matches!(acc.op, FpAccessOp::StrX | FpAccessOp::StrW) {
                    if let Some(&ty) = param_type.get(pname.as_str()) {
                        off_type.entry(acc.offset).or_insert_with(|| ty.to_string());
                    }
                }
            }
            continue;
        }

        if ln.comment.contains("signed-load") {
            if let Some(acc) = classify_fp_access(ln) {
                if acc.op == FpAccessOp::Ldrsw {
                    off_type.entry(acc.offset).or_insert_with(|| "INT32".to_string());
                }
            }
            continue;
        }

        if ln.code.is_empty() && !ln.comment.is_empty() {
            if let Some((name, ty)) = find_let_annotation(&ln.comment) {
                pending_let = Some((name, ty));
            }
            continue;
        }

        if pending_let.is_some() {
            if let Some(acc) = classify_fp_access(ln) {
                if matches!(acc.op, FpAccessOp::StrX | FpAccessOp::StrW) {
                    if !ln.comment.contains("spill param") {
                        if let Some((_, ty)) = pending_let.as_ref() {
                            off_type.entry(acc.offset).or_insert_with(|| ty.clone());
                        }
                    }
                    // Cleared unconditionally once an STR_X/STR_W access
                    // is seen, whether or not it was the "spill param"
                    // exception — mirrors Python's `pending_let = None`
                    // sitting outside the inner `if`.
                    pending_let = None;
                }
            }
        }
    }

    off_type
}

// ---------------------------------------------------------------------
// DataGlobal + parse_globals
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataGlobal {
    pub name: String,
    pub section: String,
    pub align_log2: Option<u32>,
    pub align_line: Option<u32>,
    pub label_line: u32,
    /// `(line_no, directive, args, comment)` per emitted data directive.
    pub payload: Vec<(u32, String, String, String)>,
    pub is_text_section: bool,
}

/// `^\.section\s+(\.[A-Za-z0-9_.]+)(?:\s*,.*)?\s*$`
fn match_section(code: &str) -> Option<String> {
    let after_kw = pattern::lit_req_ws(code, ".section")?;
    let after_ws = pattern::skip_ws(after_kw);
    let rest = after_ws.strip_prefix('.')?;
    let ident_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .unwrap_or(rest.len());
    if ident_len == 0 {
        return None;
    }
    let name = format!(".{}", &rest[..ident_len]);
    let after = pattern::skip_ws(&rest[ident_len..]);
    if after.is_empty() || after.starts_with(',') {
        Some(name)
    } else {
        None
    }
}

/// `^\.global\s+([A-Za-z_]\w*)\s*$`
fn match_global_decl(code: &str) -> Option<String> {
    let rest = pattern::lit_req_ws(code, ".global")?;
    let (name, rest) = pattern::ident(rest)?;
    if pattern::at_end(rest) {
        Some(name.to_string())
    } else {
        None
    }
}

/// `^\.align\s+(\d+)\s*$`
fn match_align(code: &str) -> Option<u32> {
    let rest = pattern::lit_req_ws(code, ".align")?;
    let (n, rest) = pattern::digits(rest)?;
    if pattern::at_end(rest) {
        n.parse().ok()
    } else {
        None
    }
}

const DATA_DIRECTIVES: &[&str] = &[
    "byte", "short", "hword", "word", "long", "quad", "zero", "skip", "asciz", "ascii", "string",
];

/// `^\.(byte|short|hword|word|long|quad|zero|skip|asciz|ascii|string)(?:\s+(.*))?$`
///
/// None of these eleven names is a prefix of another, so trying them in
/// any order (here, the same order as the Python alternation) can't
/// change which one matches.
fn match_data_directive(code: &str) -> Option<(String, String)> {
    let rest = code.strip_prefix('.')?;
    for &d in DATA_DIRECTIVES {
        let Some(after_name) = rest.strip_prefix(d) else { continue };
        if after_name.is_empty() {
            return Some((d.to_string(), String::new()));
        }
        if after_name.starts_with(|c: char| c.is_whitespace()) {
            let args = pattern::skip_ws(after_name);
            return Some((d.to_string(), args.trim().to_string()));
        }
    }
    None
}

/// Mirrors `parse_globals()` in `parse_s.py`.
pub fn parse_globals(s_text: &str) -> Vec<DataGlobal> {
    let lines = split_lines(s_text);
    let n = lines.len();
    let mut globals_ = Vec::new();
    let mut cur_section = ".text".to_string();

    // `lines[j].is_blank or (not lines[j].code and lines[j].comment)`
    // simplifies to just `lines[j].is_blank` — `is_blank` IS `not code`,
    // so the second disjunct (`not code and comment`) is already implied
    // by the first (`A or (A and B) == A`). Verified by inspection of the
    // `Line.is_blank` definition, not re-derived per call site.
    let skip_blank = |j: &mut usize| {
        while *j < n && lines[*j].is_blank() {
            *j += 1;
        }
    };

    let mut i = 0usize;
    while i < n {
        let code = &lines[i].code;

        if let Some(sec) = match_section(code) {
            cur_section = sec;
            i += 1;
            continue;
        }

        let Some(name) = match_global_decl(code) else {
            i += 1;
            continue;
        };

        let mut align_log2 = None;
        let mut align_line = None;
        let mut j = i + 1;
        skip_blank(&mut j);
        if j < n {
            if let Some(a) = match_align(&lines[j].code) {
                align_log2 = Some(a);
                align_line = Some(lines[j].no);
                j += 1;
                skip_blank(&mut j);
            }
        }

        let mut label_line: Option<u32> = None;
        if j < n {
            if match_label(&lines[j].code).as_deref() == Some(name.as_str()) {
                label_line = Some(lines[j].no);
                j += 1;
            }
        }

        let Some(label_line) = label_line else {
            globals_.push(DataGlobal {
                name,
                section: cur_section.clone(),
                align_log2,
                align_line,
                label_line: lines[i].no,
                payload: Vec::new(),
                is_text_section: cur_section.starts_with(".text"),
            });
            i += 1;
            continue;
        };

        let mut payload = Vec::new();
        while j < n {
            let jcode = &lines[j].code;
            if match_global_decl(jcode).is_some()
                || match_section(jcode).is_some()
                || jcode.starts_with(".end")
                || match_label(jcode).is_some()
            {
                break;
            }
            if let Some((dir, args)) = match_data_directive(jcode) {
                payload.push((lines[j].no, dir, args, lines[j].comment.clone()));
            }
            j += 1;
        }

        globals_.push(DataGlobal {
            name,
            section: cur_section.clone(),
            align_log2,
            align_line,
            label_line,
            payload,
            is_text_section: cur_section.starts_with(".text"),
        });
        i = j;
    }

    globals_
}

/// Mirrors `classify_data_global()` in `parse_s.py`.
pub fn classify_data_global(g: &DataGlobal) -> &'static str {
    if g.is_text_section {
        return "function";
    }
    if g.payload.is_empty() {
        return "empty";
    }

    if g.payload.iter().any(|(_, d, _, _)| matches!(d.as_str(), "asciz" | "ascii" | "string")) {
        return "array_const";
    }

    fn is_zero_directive(d: &str, args: &str) -> bool {
        if matches!(d, "zero" | "skip") {
            return true;
        }
        if args.contains(',') {
            args.split(',').all(|p| p.trim() == "0")
        } else {
            args.trim() == "0"
        }
    }

    let all_zero = g.payload.iter().all(|(_, d, a, _)| is_zero_directive(d, a));
    if all_zero {
        return "zero_init";
    }

    let has_multi = g.payload.iter().any(|(_, _, a, _)| a.contains(','));
    if g.payload.len() == 1 && !has_multi {
        return "scalar_const";
    }

    "array_const"
}

const BYTE_ELEMENT_HINTS: &[&str] = &["U8", "S8", "CHAR", "UINT8", "INT8"];

pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `\b(?:U8|S8|CHAR|UINT8|INT8)\b`
fn contains_word(haystack: &str, needle: &str) -> bool {
    let mut start = 0usize;
    while let Some(rel) = haystack[start..].find(needle) {
        let idx = start + rel;
        let before_ok = haystack[..idx].chars().next_back().map(|c| !is_word_char(c)).unwrap_or(true);
        let after_ok = haystack[idx + needle.len()..]
            .chars()
            .next()
            .map(|c| !is_word_char(c))
            .unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        start = idx + 1;
    }
    false
}

/// Mirrors `is_byte_array_global()` in `parse_s.py`.
pub fn is_byte_array_global(g: &DataGlobal) -> bool {
    if g.payload.is_empty() {
        return false;
    }
    if g
        .payload
        .iter()
        .any(|(_, d, _, _)| matches!(d.as_str(), "byte" | "asciz" | "ascii" | "string"))
    {
        return true;
    }
    g.payload
        .iter()
        .any(|(_, _, _, c)| BYTE_ELEMENT_HINTS.iter().any(|h| contains_word(c, h)))
}

// ---------------------------------------------------------------------
// Signed-branch / compare site detection
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmpSignedBranch {
    pub cmp_line_no: u32,
    pub cmp_lhs: String,
    pub cmp_rhs: String,
    pub branch_line_no: u32,
    pub branch_suffix: String,
}

const SIGNED_BRANCH_SUFFIXES: &[&str] = &["LT", "LE", "GT", "GE"];

/// `^B\.(LT|LE|GT|GE)\b`
fn match_signed_branch(code: &str) -> Option<&'static str> {
    let rest = code.strip_prefix("B.")?;
    for &suf in SIGNED_BRANCH_SUFFIXES {
        if let Some(after) = rest.strip_prefix(suf) {
            let boundary_ok = after.chars().next().map(|c| !is_word_char(c)).unwrap_or(true);
            if boundary_ok {
                return Some(suf);
            }
        }
    }
    None
}

/// `^CMP\s+(X\w+)\s*,\s*(X\w+|XZR|#-?\d+)\s*$` — the `X\w+|XZR`
/// alternation collapses to just `X\w+` (see [`pattern::reg_token`]).
fn match_cmp_x(code: &str) -> Option<(String, String)> {
    let rest = pattern::lit_req_ws(code, "CMP")?;
    let (lhs, rest) = pattern::reg_token(rest, 'X')?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::skip_ws(rest);
    if let Some((rhs, after)) = pattern::reg_token(rest, 'X') {
        if pattern::at_end(after) {
            return Some((lhs.to_uppercase(), rhs.to_uppercase()));
        }
    }
    if let Some(after_hash) = rest.strip_prefix('#') {
        if let Some((num, after)) = pattern::signed_digits(after_hash) {
            if pattern::at_end(after) {
                return Some((lhs.to_uppercase(), format!("#{num}")));
            }
        }
    }
    None
}

/// Mirrors `find_signed_compare_sites()` in `parse_s.py`: for every
/// `B.LT`/`B.LE`/`B.GT`/`B.GE` in the function body, scans up to 8 lines
/// backward for the nearest `CMP Xn, ...` and pairs them.
pub fn find_signed_compare_sites(fn_: &Function) -> Vec<CmpSignedBranch> {
    let body = &fn_.body;
    let mut out = Vec::new();
    for (idx, ln) in body.iter().enumerate() {
        let Some(suffix) = match_signed_branch(&ln.code) else { continue };

        let mut found: Option<(u32, String, String)> = None;
        for back in 1..=8usize {
            if back > idx {
                break;
            }
            let prev = &body[idx - back];
            if let Some((lhs, rhs)) = match_cmp_x(&prev.code) {
                found = Some((prev.no, lhs, rhs));
                break;
            }
        }

        if let Some((cmp_line_no, cmp_lhs, cmp_rhs)) = found {
            out.push(CmpSignedBranch {
                cmp_line_no,
                cmp_lhs,
                cmp_rhs,
                branch_line_no: ln.no,
                branch_suffix: suffix.to_string(),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------
// find_recent_x_def
// ---------------------------------------------------------------------

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        s.get(prefix.len()..)
    } else {
        None
    }
}

/// Mirrors the dynamically-built `re.compile(rf"^[A-Z]+S?W?\s+{re.escape(target)}\s*,", re.IGNORECASE)`
/// in `find_recent_x_def`. The `S?W?` suffix in the Python pattern is
/// vestigial: `[A-Z]+` is greedy (and, being case-insensitive, matches
/// any letters), so it always consumes any trailing S/W letters itself
/// before the mandatory `\s+` — `S?`/`W?` end up matching zero-width in
/// every case, since they contribute nothing when empty and there's no
/// backtracking incentive to leave characters for them. So this reduces
/// to: one-or-more letters, mandatory whitespace, the target register
/// (case-insensitive), optional whitespace, a comma.
fn defines_register(code: &str, target: &str) -> bool {
    let after_mnemonic = match code.find(|c: char| !c.is_ascii_alphabetic()) {
        Some(0) | None => return false,
        Some(i) => &code[i..],
    };
    if !after_mnemonic.starts_with(|c: char| c.is_whitespace()) {
        return false;
    }
    let after_ws = pattern::skip_ws(after_mnemonic);
    let Some(after_target) = strip_prefix_ignore_case(after_ws, target) else { return false };
    pattern::skip_ws(after_target).starts_with(',')
}

/// Mirrors `find_recent_x_def()` in `parse_s.py`: scans up to `window`
/// lines backward from the line numbered `before_line_no` for the nearest
/// instruction whose first operand is `target_reg` (or its W-view).
/// Returns `(line_no, code)` of the definition, if found within the
/// window.
pub fn find_recent_x_def(
    fn_: &Function,
    target_reg: &str,
    before_line_no: u32,
    window: usize,
) -> Option<(u32, String)> {
    let body = &fn_.body;
    let target_idx = body.iter().position(|ln| ln.no == before_line_no)?;

    let target_x = target_reg.to_uppercase();
    let target_w = format!("W{}", &target_x[1..]);

    for back in 1..=window {
        if back > target_idx {
            break;
        }
        let i = target_idx - back;
        let code = &body[i].code;
        if code.is_empty() {
            continue;
        }
        if defines_register(code, &target_x) || defines_register(code, &target_w) {
            return Some((body[i].no, code.clone()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_lines_matches_python_splitlines_edge_cases() {
        assert_eq!(split_lines("").len(), 0);
        assert_eq!(split_lines("a").iter().map(|l| l.code.as_str()).collect::<Vec<_>>(), vec!["a"]);
        assert_eq!(split_lines("a\n").iter().map(|l| l.code.as_str()).collect::<Vec<_>>(), vec!["a"]);
        assert_eq!(
            split_lines("a\nb").iter().map(|l| l.code.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(
            split_lines("a\n\n").iter().map(|l| l.code.as_str()).collect::<Vec<_>>(),
            vec!["a", ""]
        );
        assert_eq!(
            split_lines("a\r\nb\r\n").iter().map(|l| l.code.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn comment_split_is_naive_about_urls_matching_python() {
        let lines = split_lines(".asciz \"http://x\"");
        assert_eq!(lines[0].code, ".asciz \"http:");
        assert_eq!(lines[0].comment, "x\"");
    }

    #[test]
    fn type_function_requires_mandatory_whitespace() {
        assert_eq!(match_type_function(".type foo, %function"), Some("foo".to_string()));
        assert_eq!(match_type_function(".typefoo, %function"), None);
        assert_eq!(match_type_function(".type  spaced_fn ,  %function "), Some("spaced_fn".to_string()));
    }

    #[test]
    fn end_of_file_and_next_type_both_drop_final_body_line() {
        // Empirically confirmed against the real Python parse_s.py before
        // porting: when a function has no `.size`, the very last line of
        // the file is reported as `end_line` but excluded from `body`.
        let text = ".type _entry, %function\n_entry:\n    MOV SP, X0\n    RET\n";
        let fns = parse_functions(text);
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].end_line, 4);
        assert_eq!(fns[0].body.iter().map(|l| l.no).collect::<Vec<_>>(), vec![3]);

        let text2 = ".type first_fn, %function\nfirst_fn:\n    MOV X0, #1\n    RET\n.type second_fn, %function\nsecond_fn:\n    MOV X0, #2\n    RET\n.size second_fn, . - second_fn\n";
        let fns2 = parse_functions(text2);
        assert_eq!(fns2[0].name, "first_fn");
        assert_eq!(fns2[0].end_line, 4);
        assert_eq!(fns2[0].body.iter().map(|l| l.no).collect::<Vec<_>>(), vec![3]);
        assert_eq!(fns2[1].name, "second_fn");
        assert_eq!(fns2[1].body.iter().map(|l| l.no).collect::<Vec<_>>(), vec![7, 8]);
    }

    #[test]
    fn params_comment_and_pairs() {
        let (inner, ret) =
            match_params_comment("params: [('a', 'INT32'), ('b', 'U64')] ret: VOID").unwrap();
        assert_eq!(ret, "VOID");
        let pairs = find_param_pairs(&inner);
        assert_eq!(
            pairs,
            vec![("a".to_string(), "INT32".to_string()), ("b".to_string(), "U64".to_string())]
        );
    }

    #[test]
    fn assert_parse_complete_reports_dropped_type_declaration() {
        let text = ".type ghost_fn, %function\n    NOP\n    NOP\n    NOP\n    NOP\nghost_fn:\n    RET\n";
        let err = assert_parse_complete(text, "weird.s").unwrap_err();
        assert_eq!(
            err.0,
            "weird.s: parse_functions silently dropped 1 function-typed declaration(s): ['ghost_fn']. \
First offender: `.type ghost_fn, %function` at weird.s:1. \
Refusing to emit K-check verdicts on a partial parse."
        );
    }

    #[test]
    fn classify_fp_access_prefers_xzr_over_generic_x() {
        let ln = Line { no: 1, raw: "STR XZR, [X29, #8]".into(), code: "STR XZR, [X29, #8]".into(), comment: String::new() };
        let acc = classify_fp_access(&ln).unwrap();
        assert_eq!(acc.op, FpAccessOp::StrXzr);
        assert_eq!(acc.offset, 8);
    }

    #[test]
    fn spill_push_excludes_frame_registers() {
        let ln = |code: &str| Line { no: 1, raw: code.into(), code: code.into(), comment: String::new() };
        assert!(classify_sp_adjust(&ln("STR X9, [SP, #-16]!")).is_some());
        assert_eq!(classify_sp_adjust(&ln("STR X29, [SP, #-16]!")), None);
        assert_eq!(classify_sp_adjust(&ln("STR X30, [SP, #-16]!")), None);
    }

    #[test]
    fn signed_branch_requires_word_boundary() {
        assert_eq!(match_signed_branch("B.LT some_label"), Some("LT"));
        assert_eq!(match_signed_branch("B.LTX some_label"), None);
    }

    #[test]
    fn recent_x_def_matches_w_view_and_requires_first_operand() {
        let body = vec![
            Line { no: 1, raw: "LDR W9, [X29, #8]".into(), code: "LDR W9, [X29, #8]".into(), comment: String::new() },
            Line { no: 2, raw: "CMP X9, X10".into(), code: "CMP X9, X10".into(), comment: String::new() },
        ];
        let fn_ = Function { name: "f".into(), start_line: 0, end_line: 3, body, params: vec![], ret_type: String::new() };
        let found = find_recent_x_def(&fn_, "X9", 2, 12).unwrap();
        assert_eq!(found.0, 1);
    }
}
