//! Port of `k_invariants.py`.
//!
//! The six K-invariants as pure Rust predicates over `.s` text.
//!
//! Per UFA Bare-Metal Canon brief §2.6: this is the ONE catalog file.
//! Adding a K means adding one function here and one [`K_REGISTRY`] entry
//! — no subclass hierarchies, no plugin loading. Preserved from the Python
//! original (which enforces the same discipline).
//!
//! Each K-check signature: `fn(s_text: &str, ctx: &Ctx) -> Vec<Verdict>`.
//! `ctx` carries the source filename (for diagnostics); returning an empty
//! `Vec` is equivalent to NOT_APPLICABLE.
//!
//! Function names here are `snake_case` (`k01_stack_frame_discipline`)
//! rather than the Python original's `K01_stack_frame_discipline` — purely
//! a casing convention change to match idiomatic Rust; nothing about the
//! check logic differs because of it.

use std::collections::HashSet;

use crate::parse_s::{self, DataGlobal, Function, FpAccessOp, SpAdjust, SpAdjustKind};
use crate::pattern;
use crate::verdict::{Status, Verdict};

/// Per-invocation context. Mirrors the Python `ctx: dict` parameter, which
/// in practice only ever carries `ctx["file"]`.
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    pub file: Option<String>,
}

impl Ctx {
    pub fn with_file(file: impl Into<String>) -> Self {
        Ctx { file: Some(file.into()) }
    }
}

fn vfile(file: Option<&str>) -> Option<String> {
    file.map(String::from)
}

// =====================================================================
// K01 — stack frame discipline
// =====================================================================

/// Statement (UFA Bare-Metal Canon §4 K01): every function has a balanced
/// `SUB SP, SP, #N` / `ADD SP, SP, #N` pair; every `STR XZR` / `STP XZR,
/// XZR` to a stack slot is at an 8-byte-aligned offset; frame size is a
/// multiple of 16 (AArch64 PCS).
pub fn k01_stack_frame_discipline(s_text: &str, ctx: &Ctx) -> Vec<Verdict> {
    let file = ctx.file.as_deref();
    let mut verdicts = Vec::new();
    for fn_ in parse_s::parse_functions(s_text) {
        verdicts.extend(k01_check_function(&fn_, file));
    }
    verdicts
}

fn k01_check_function(fn_: &Function, file: Option<&str>) -> Vec<Verdict> {
    let mut out: Vec<Verdict> = Vec::new();
    let sp = parse_s::collect_sp_adjusts(fn_);

    let sub_total: u64 = sp.iter().filter(|a| a.kind == SpAdjustKind::SubSp).map(|a| a.amount).sum();
    let add_total: u64 = sp.iter().filter(|a| a.kind == SpAdjustKind::AddSp).map(|a| a.amount).sum();
    let pushes: Vec<&SpAdjust> = sp.iter().filter(|a| a.kind == SpAdjustKind::SpillPush).collect();
    let pops: Vec<&SpAdjust> = sp.iter().filter(|a| a.kind == SpAdjustKind::SpillPop).collect();
    let stp_prologue_count = sp.iter().filter(|a| a.kind == SpAdjustKind::StpPrologue).count();
    let ldp_epilogue_count = sp.iter().filter(|a| a.kind == SpAdjustKind::LdpEpilogue).count();

    // 1. Frame allocation must equal frame deallocation.
    if sub_total != add_total {
        out.push(
            Verdict::new(Status::Fail, "K01")
                .file_opt(vfile(file))
                .function(fn_.name.clone())
                .line_no(fn_.start_line)
                .diagnostic(format!(
                    "frame imbalance: SUB SP totals {sub_total} bytes, ADD SP totals {add_total} bytes"
                ))
                .suggested_fix(
                    "Make sure every prologue `SUB SP, SP, #N(, LSL #12)?` is matched by an \
epilogue `ADD SP, SP, #N(, LSL #12)?` with the same total. See Bug #1, Bug #19, Bug #20.",
                ),
        );
    }

    // 2. Frame allocation must be a multiple of 16 (AArch64 PCS).
    if sub_total > 0 && sub_total % 16 != 0 {
        out.push(
            Verdict::new(Status::Fail, "K01")
                .file_opt(vfile(file))
                .function(fn_.name.clone())
                .line_no(fn_.start_line)
                .diagnostic(format!(
                    "frame size {sub_total} bytes is not a multiple of 16 (AArch64 PCS requires SP to remain 16-byte aligned)"
                ))
                .suggested_fix("Round local frame allocation up to a multiple of 16."),
        );
    }

    // 3. Spill push/pop balance (excluding the X29/X30 STP/LDP pair).
    if pushes.len() != pops.len() {
        let first_offender = if pushes.len() > pops.len() { pushes[0] } else { pops[0] };
        out.push(
            Verdict::new(Status::Fail, "K01")
                .file_opt(vfile(file))
                .function(fn_.name.clone())
                .line_no(first_offender.line_no)
                .diagnostic(format!(
                    "unbalanced register-spill stack writes: {} `STR Xn, [SP, #-16]!` push(es) vs {} `LDR Xn, [SP], #16` pop(s)",
                    pushes.len(),
                    pops.len()
                ))
                .suggested_fix(
                    "Every `STR Xn, [SP, #-16]!` must be matched by a `LDR Xn, [SP], #16` within \
the same function. See Bug #12, Bug #23 — the nested-binop save/restore pattern.",
                ),
        );
    }

    // 4. The X29/X30 STP/LDP prologue/epilogue must come in matched pairs.
    if stp_prologue_count != ldp_epilogue_count {
        out.push(
            Verdict::new(Status::Fail, "K01")
                .file_opt(vfile(file))
                .function(fn_.name.clone())
                .line_no(fn_.start_line)
                .diagnostic(format!(
                    "X29/X30 prologue/epilogue mismatch: {stp_prologue_count} STP X29,X30 / {ldp_epilogue_count} LDP X29,X30"
                ))
                .suggested_fix(
                    "Every function must save FP+LR with `STP X29, X30, [SP, #-16]!` at entry and \
restore with `LDP X29, X30, [SP], #16` before RET.",
                ),
        );
    }

    // 5. STR XZR / STR Xn / STP at unaligned X29-relative offsets (Bug #54).
    for acc in parse_s::collect_fp_accesses(fn_) {
        match acc.op {
            FpAccessOp::StrXzr if acc.offset % 8 != 0 => {
                out.push(
                    Verdict::new(Status::Fail, "K01")
                        .file_opt(vfile(file))
                        .function(fn_.name.clone())
                        .line_no(acc.line_no)
                        .diagnostic(format!(
                            "`STR XZR, [X29, #{}]` at non-8-aligned offset (Bug #54: hangs cortex-a53 with SCTLR_EL1.A=1)",
                            acc.offset
                        ))
                        .suggested_fix(
                            "Decompose unaligned 8B zero-stores into two `STR WZR, [X29, #cur+0]` / \
`STR WZR, [X29, #cur+4]`. See Bug #54 fix in `_emit_zero_block`.",
                        ),
                );
            }
            FpAccessOp::StrX if acc.offset % 8 != 0 => {
                out.push(
                    Verdict::new(Status::Fail, "K01")
                        .file_opt(vfile(file))
                        .function(fn_.name.clone())
                        .line_no(acc.line_no)
                        .diagnostic(format!(
                            "`STR {}, [X29, #{}]` at non-8-aligned offset (8-byte store requires 8-byte-aligned address)",
                            acc.reg, acc.offset
                        ))
                        .suggested_fix("Round the X29-relative offset up to a multiple of 8."),
                );
            }
            FpAccessOp::Stp if acc.offset % 8 != 0 => {
                out.push(
                    Verdict::new(Status::Fail, "K01")
                        .file_opt(vfile(file))
                        .function(fn_.name.clone())
                        .line_no(acc.line_no)
                        .diagnostic(format!(
                            "`STP {}, {}, [X29, #{}]` at non-8-aligned offset",
                            acc.reg, acc.reg2, acc.offset
                        ))
                        .suggested_fix("STP/LDP requires 8-byte-aligned base offset."),
                );
            }
            _ => {}
        }
    }

    if out.is_empty() {
        out.push(
            Verdict::new(Status::Pass, "K01")
                .file_opt(vfile(file))
                .function(fn_.name.clone())
                .line_no(fn_.start_line)
                .diagnostic("frame, spill discipline, and offset alignment OK"),
        );
    }
    out
}

// =====================================================================
// K02 — register spill balance (+ tuple-receive ordering micro-check)
// =====================================================================

/// Statement (UFA Bare-Metal Canon §4 K02): every emitted scratch register
/// used in nested binop / call sequences is preserved across child
/// operations that may clobber it, via a balanced `STR Xn, [SP, #-16]! ...
/// LDR Xn, [SP], #16` save/restore.
pub fn k02_register_spill_balance(s_text: &str, ctx: &Ctx) -> Vec<Verdict> {
    let file = ctx.file.as_deref();
    let mut verdicts = Vec::new();
    for fn_ in parse_s::parse_functions(s_text) {
        verdicts.extend(k02_check_function(&fn_, file));
    }
    verdicts
}

/// `^(?:BL|BLR)\b`. "BL" is a genuine prefix of "BLR" (unlike the
/// vestigial alternations elsewhere in this codebase), so this DOES need
/// to try both alternatives with a real boundary check, not just the
/// longer/shorter one.
fn matches_tuple_recv_bl(code: &str) -> bool {
    for mnemonic in ["BL", "BLR"] {
        if let Some(after) = code.strip_prefix(mnemonic) {
            let boundary_ok = after.chars().next().map(|c| !parse_s::is_word_char(c)).unwrap_or(true);
            if boundary_ok {
                return true;
            }
        }
    }
    false
}

fn matches_str_x0_fp(code: &str) -> bool {
    parse_s::match_fixed_reg_base_offset(code, "STR", "X0", "X29").is_some()
}

fn matches_str_x1_fp(code: &str) -> bool {
    parse_s::match_fixed_reg_base_offset(code, "STR", "X1", "X29").is_some()
}

/// K02 v2 micro-check: tuple-receive store ordering. Brief §3 Stable
/// Invariant #8: for each `BL/BLR` whose return shape is the X0+X1
/// tuple (recognized by an immediately-following pair of `STR X0/X1,
/// [X29, ...]` stores), the X1 store MUST precede the X0 store.
fn k02_check_tuple_recv_ordering(fn_: &Function, file: Option<&str>) -> Vec<Verdict> {
    let mut out = Vec::new();
    let body = &fn_.body;

    for (idx, ln) in body.iter().enumerate() {
        if !matches_tuple_recv_bl(&ln.code) {
            continue;
        }
        let mut x0_idx: Option<usize> = None;
        let mut x1_idx: Option<usize> = None;
        for fwd in 1..=6usize {
            if idx + fwd >= body.len() {
                break;
            }
            let cand = &body[idx + fwd];
            if cand.code.is_empty() {
                continue;
            }
            let is_x0 = matches_str_x0_fp(&cand.code);
            let is_x1 = matches_str_x1_fp(&cand.code);
            if x0_idx.is_none() && is_x0 {
                x0_idx = Some(idx + fwd);
                continue;
            }
            if x1_idx.is_none() && is_x1 {
                x1_idx = Some(idx + fwd);
                continue;
            }
            if !is_x0 && !is_x1 {
                break;
            }
        }

        if let (Some(x0i), Some(x1i)) = (x0_idx, x1_idx) {
            if x0i < x1i {
                out.push(
                    Verdict::new(Status::Fail, "K02")
                        .file_opt(vfile(file))
                        .function(fn_.name.clone())
                        .line_no(body[x0i].no)
                        .diagnostic(format!(
                            "tuple-receive after `BL/BLR` at line {}: `STR X0, [X29, ...]` at line {} \
precedes `STR X1, [X29, ...]` at line {}. Brief §3 Stable Invariant #8 requires X1 first \
(forward-safety against future addressing-mode helpers using W1).",
                            ln.no, body[x0i].no, body[x1i].no
                        ))
                        .suggested_fix(
                            "Reorder the tuple-receive stores so STR X1 precedes STR X0. See \
_emit_tuple_recv at compiler/core/aarch64_assembler.py:2147.",
                        ),
                );
            }
        }
    }
    out
}

fn k02_check_function(fn_: &Function, file: Option<&str>) -> Vec<Verdict> {
    let mut out: Vec<Verdict> = Vec::new();
    let sp = parse_s::collect_sp_adjusts(fn_);

    let pushes: Vec<&SpAdjust> = sp.iter().filter(|a| a.kind == SpAdjustKind::SpillPush).collect();
    let pops: Vec<&SpAdjust> = sp.iter().filter(|a| a.kind == SpAdjustKind::SpillPop).collect();

    // 1. Total count balance.
    if pushes.len() != pops.len() {
        let offender_line =
            if pushes.len() > pops.len() { pushes.last().unwrap().line_no } else { pops.last().unwrap().line_no };
        out.push(
            Verdict::new(Status::Fail, "K02")
                .file_opt(vfile(file))
                .function(fn_.name.clone())
                .line_no(offender_line)
                .diagnostic(format!(
                    "unbalanced scratch-register spill: {} `STR Xn, [SP, #-16]!` vs {} `LDR Xn, [SP], #16`. Bug #12-class regression.",
                    pushes.len(),
                    pops.len()
                ))
                .suggested_fix(
                    "Every nested-binop spill `STR Xn, [SP, #-16]!` must be matched by a `LDR Xn, \
[SP], #16` later in the same function, before the function returns.",
                ),
        );
    }

    // 2. Running depth.
    let mut depth: i64 = 0;
    for a in &sp {
        match a.kind {
            SpAdjustKind::SpillPush => depth += 1,
            SpAdjustKind::SpillPop => {
                depth -= 1;
                if depth < 0 {
                    out.push(
                        Verdict::new(Status::Fail, "K02")
                            .file_opt(vfile(file))
                            .function(fn_.name.clone())
                            .line_no(a.line_no)
                            .diagnostic(format!(
                                "spill stack underflow at `LDR {}, [SP], #16`: `LDR Xn, [SP], #16` \
with no outstanding `STR Xn, [SP, #-16]!` to consume",
                                a.reg
                            ))
                            .suggested_fix(
                                "Each `LDR Xn, [SP], #16` consumes a 16B slot; the compiler must \
emit a matching `STR Xn, [SP, #-16]!` BEFORE this line. See Bug #23 fix in `_emit_simple_cmp`.",
                            ),
                    );
                    depth = 0;
                }
            }
            _ => {}
        }
    }

    // 3. End-of-function depth must be zero at every LDP_EPILOGUE.
    let mut depth2: i64 = 0;
    for a in &sp {
        match a.kind {
            SpAdjustKind::SpillPush => depth2 += 1,
            SpAdjustKind::SpillPop => depth2 -= 1,
            SpAdjustKind::LdpEpilogue if depth2 != 0 => {
                out.push(
                    Verdict::new(Status::Fail, "K02")
                        .file_opt(vfile(file))
                        .function(fn_.name.clone())
                        .line_no(a.line_no)
                        .diagnostic(format!(
                            "function exit reached with non-zero spill depth ({depth2}); SP would be off by {} bytes after RET",
                            depth2 * 16
                        ))
                        .suggested_fix(
                            "Drain all outstanding `STR Xn, [SP, #-16]!` before the function's \
`LDP X29, X30, [SP], #16` epilogue.",
                        ),
                );
            }
            _ => {}
        }
    }

    out.extend(k02_check_tuple_recv_ordering(fn_, file));

    if out.is_empty() {
        out.push(
            Verdict::new(Status::Pass, "K02")
                .file_opt(vfile(file))
                .function(fn_.name.clone())
                .line_no(fn_.start_line)
                .diagnostic(format!("scratch-register spill discipline OK ({} push/pop pair(s))", pushes.len())),
        );
    }
    out
}

// =====================================================================
// K03 — type-width load/store discipline
// =====================================================================

const INT32_TYPES: [&str; 2] = ["INT32", "S32"];
// `_UINT32_TYPES` in the Python original is defined but never referenced
// by either K03 or K04's actual check logic — a vestigial constant, not
// ported here since an unused Rust `const` would just trigger a
// dead-code warning for no behavioral benefit.
const INT64_TYPES: [&str; 2] = ["INT64", "S64"];
const UINT64_TYPES: [&str; 2] = ["UINT64", "U64"];

/// Statement (UFA Bare-Metal Canon §4 K03): loads of INT32/S32 use
/// `LDRSW` (sign-extend); loads of UINT32/U32 use `LDR W` (zero-extend);
/// INT32×INT32→INT64 multiply uses `SMULL`, not `MUL`; UINT32×UINT32→
/// UINT64 uses `UMULL`.
pub fn k03_type_width_load_store(s_text: &str, ctx: &Ctx) -> Vec<Verdict> {
    let file = ctx.file.as_deref();
    let mut verdicts = Vec::new();
    for fn_ in parse_s::parse_functions(s_text) {
        verdicts.extend(k03_check_function(&fn_, file));
    }
    verdicts
}

/// `^MUL\s+(X\w+)\s*,\s*(X\w+)\s*,\s*(X\w+)\s*$` — bare `MUL Xd, Xn, Xm`
/// (no S/U widen). Anchoring on an X-register first operand keeps this
/// from matching `SMULL`/`UMULL`/`SMULH` lines (checked separately, and
/// first, in the caller).
fn match_mul_x(code: &str) -> Option<(String, String, String)> {
    let rest = pattern::lit_req_ws(code, "MUL")?;
    let (rd, rest) = pattern::reg_token(rest, 'X')?;
    let rest = pattern::lit(rest, ",")?;
    let (rn, rest) = pattern::reg_token(rest, 'X')?;
    let rest = pattern::lit(rest, ",")?;
    let (rm, rest) = pattern::reg_token(rest, 'X')?;
    if pattern::at_end(rest) {
        Some((rd.to_uppercase(), rn.to_uppercase(), rm.to_uppercase()))
    } else {
        None
    }
}

/// `^LDR\s+(X\w+)\s*,\s*\[\s*(X\w+)\s*,\s*(X\w+)\s*,\s*LSL\s+#(\d+)\s*\]\s*$`
fn match_ldr_x_lsl(code: &str) -> Option<(String, String, String, u32)> {
    let rest = pattern::lit_req_ws(code, "LDR")?;
    let (rd, rest) = pattern::reg_token(rest, 'X')?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "[")?;
    let (rn, rest) = pattern::reg_token(rest, 'X')?;
    let rest = pattern::lit(rest, ",")?;
    let (rm, rest) = pattern::reg_token(rest, 'X')?;
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit_req_ws(rest, "LSL")?;
    let rest = pattern::lit(rest, "#")?;
    let (shift_str, rest) = pattern::digits(rest)?;
    let rest = pattern::lit(rest, "]")?;
    if pattern::at_end(rest) {
        Some((rd.to_uppercase(), rn.to_uppercase(), rm.to_uppercase(), shift_str.parse().ok()?))
    } else {
        None
    }
}

/// `^(?:SMULL|UMULL|SMADDL|UMADDL)\b` — none of these four is a prefix of
/// another, so trying them in any order can't change which one matches.
fn matches_smull_family(code: &str) -> bool {
    for mnemonic in ["SMULL", "UMULL", "SMADDL", "UMADDL"] {
        if let Some(after) = code.strip_prefix(mnemonic) {
            let boundary_ok = after.chars().next().map(|c| !parse_s::is_word_char(c)).unwrap_or(true);
            if boundary_ok {
                return true;
            }
        }
    }
    false
}

fn k03_check_function(fn_: &Function, file: Option<&str>) -> Vec<Verdict> {
    let mut out: Vec<Verdict> = Vec::new();
    let off_type = parse_s::build_offset_type_map(fn_);
    let mut seen_failure_offsets: HashSet<i64> = HashSet::new();

    // K03 v2 micro-check: register-offset LDR with LSL shift > 3 (Bug #43).
    // Iterated first since it's order-independent of the rest.
    for ln in &fn_.body {
        if let Some((rd, rn, rm, shift)) = match_ldr_x_lsl(&ln.code) {
            if shift > 3 {
                out.push(
                    Verdict::new(Status::Fail, "K03")
                        .file_opt(vfile(file))
                        .function(fn_.name.clone())
                        .line_no(ln.no)
                        .diagnostic(format!(
                            "`LDR {rd}, [{rn}, {rm}, LSL #{shift}]` — value-load shift exceeds \
AArch64 X-width bound (LSL #3 = 8-byte stride). Bug #43-class regression: struct-elem field \
stride {} bytes requires `ADD address, ...` not `LDR LSL`.",
                            1u64 << shift
                        ))
                        .suggested_fix(format!(
                            "Replace the LDR-with-shift with explicit address materialization: \
`ADD Xa, base, idx, LSL #{shift}` then `LDR/STR Xa, [Xa]`. See _emit_struct_base_addr 5d branch \
in compiler/core/aarch64_assembler.py."
                        )),
                );
            }
        }
    }

    // 1. LDR X / LDRSW / LDR W vs declared type.
    for ln in &fn_.body {
        let Some(acc) = parse_s::classify_fp_access(ln) else { continue };
        let Some(decl) = off_type.get(&acc.offset) else { continue };

        if acc.op == FpAccessOp::LdrX && INT32_TYPES.contains(&decl.as_str()) {
            if seen_failure_offsets.contains(&acc.offset) {
                continue;
            }
            seen_failure_offsets.insert(acc.offset);
            out.push(
                Verdict::new(Status::Fail, "K03")
                    .file_opt(vfile(file))
                    .function(fn_.name.clone())
                    .line_no(acc.line_no)
                    .diagnostic(format!(
                        "`LDR {}, [X29, #{}]` reads a {decl} local using LDR X (no sign-extension); \
upper 32 bits will be stale. Bug #49 regression.",
                        acc.reg, acc.offset
                    ))
                    .suggested_fix(format!(
                        "Use `LDRSW {}, [X29, #{}]` to sign-extend the {decl} value to 64 bits \
before any subsequent X-width op.",
                        acc.reg, acc.offset
                    )),
            );
        }
        // LDRSW with a non-INT32 declared type is suspicious but is
        // deliberately not flagged here (false-positive risk on
        // legitimate manual narrowing) — matches the Python original's
        // own comment on this exact point.
    }

    // 2. Bare MUL where context demands SMULL/UMULL. Linear state machine:
    // a `// let NAME: TYPE = ... * ...` comment sets a pending mul-let,
    // which the next SMULL/UMULL/SMADDL/UMADDL or bare MUL consumes.
    let mut pending_mul_let_type: Option<String> = None;
    for ln in &fn_.body {
        if ln.code.is_empty() && !ln.comment.is_empty() {
            if ln.comment.contains("let ") && ln.comment.contains(" * ") {
                if let Some((_name, ty)) = parse_s::find_let_annotation(&ln.comment) {
                    pending_mul_let_type = Some(ty);
                }
            }
            continue;
        }

        if matches_smull_family(&ln.code) {
            pending_mul_let_type = None;
            continue;
        }

        if match_mul_x(&ln.code).is_some() {
            if let Some(ty) = &pending_mul_let_type {
                if INT64_TYPES.contains(&ty.as_str()) {
                    out.push(
                        Verdict::new(Status::Fail, "K03")
                            .file_opt(vfile(file))
                            .function(fn_.name.clone())
                            .line_no(ln.no)
                            .diagnostic(
                                "`MUL` emitted where INT64 widening multiply required. Bug #3 \
regression (use SMULL Xd, Wn, Wm).",
                            )
                            .suggested_fix(
                                "Replace `MUL Xd, Xn, Xm` with `SMULL Xd, Wn, Wm` for INT32×INT32→INT64.",
                            ),
                    );
                } else if UINT64_TYPES.contains(&ty.as_str()) {
                    out.push(
                        Verdict::new(Status::Fail, "K03")
                            .file_opt(vfile(file))
                            .function(fn_.name.clone())
                            .line_no(ln.no)
                            .diagnostic(
                                "`MUL` emitted where UINT64 widening multiply required. Bug #44 \
regression (use UMULL Xd, Wn, Wm).",
                            )
                            .suggested_fix(
                                "Replace `MUL Xd, Xn, Xm` with `UMULL Xd, Wn, Wm` for UINT32×UINT32→UINT64.",
                            ),
                    );
                }
            }
            pending_mul_let_type = None;
            continue;
        }
    }

    if out.is_empty() {
        out.push(
            Verdict::new(Status::Pass, "K03")
                .file_opt(vfile(file))
                .function(fn_.name.clone())
                .line_no(fn_.start_line)
                .diagnostic(format!("type-width load/store discipline OK ({} typed offset(s))", off_type.len())),
        );
    }
    out
}

// =====================================================================
// K04 — sign extension before signed compare
// =====================================================================

/// `^LDR\s+W(\w+)\s*,\s*\[\s*X29\s*(?:,\s*#(-?\d+))?\s*\]\s*$` — note the
/// captured group is the digits AFTER the literal "W" (e.g. `"9"` for
/// `LDR W9, ...`), unlike `classify_fp_access`'s LDR_W handling which
/// captures the whole `W9` token.
fn match_ldr_w_from_fp(code: &str) -> Option<(String, i64)> {
    let rest = pattern::lit_req_ws(code, "LDR")?;
    let rest = pattern::skip_ws(rest);
    let rest = rest.strip_prefix('W')?;
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    let digits_part = rest[..end].to_string();
    let rest = &rest[end..];
    let rest = pattern::lit(rest, ",")?;
    let rest = pattern::lit(rest, "[")?;
    let rest = pattern::lit(rest, "X29")?;
    let (offset, after_offset) = parse_s::try_offset_suffix(rest).unwrap_or((0, rest));
    let rest = pattern::lit(after_offset, "]")?;
    if pattern::at_end(rest) {
        Some((digits_part, offset))
    } else {
        None
    }
}

/// Statement (UFA Bare-Metal Canon §4 K04): sub-word values flowing into a
/// signed comparison are sign-extended to 64 bits before the `CMP`.
///
/// Implementation (v1, narrow): for each `(CMP Xa, Xb) ; B.<signed>` pair,
/// walk back to find the most recent definition of Xa/Xb. If it's
/// `LDR Wn, [X29, #off]` (zero-extending) AND the offset's declared type
/// is INT32/S32, FAIL.
pub fn k04_sign_extension(s_text: &str, ctx: &Ctx) -> Vec<Verdict> {
    let file = ctx.file.as_deref();
    let mut verdicts = Vec::new();
    for fn_ in parse_s::parse_functions(s_text) {
        verdicts.extend(k04_check_function(&fn_, file));
    }
    verdicts
}

fn k04_check_function(fn_: &Function, file: Option<&str>) -> Vec<Verdict> {
    let mut out: Vec<Verdict> = Vec::new();
    let off_type = parse_s::build_offset_type_map(fn_);
    let sites = parse_s::find_signed_compare_sites(fn_);

    let mut seen_failures: HashSet<(u32, String)> = HashSet::new();

    for site in &sites {
        for operand in [&site.cmp_lhs, &site.cmp_rhs] {
            if !operand.starts_with('X') {
                continue; // immediate operand or XZR
            }
            if operand.as_str() == "XZR" {
                continue;
            }
            let Some((def_line_no, def_code)) =
                parse_s::find_recent_x_def(fn_, operand, site.cmp_line_no, 12)
            else {
                continue;
            };

            let Some((w_digits, off)) = match_ldr_w_from_fp(&def_code) else {
                continue; // def isn't a W-width frame load -> not the target shape
            };

            let Some(decl) = off_type.get(&off) else { continue };
            if !INT32_TYPES.contains(&decl.as_str()) {
                continue; // only fire when the slot was logically signed-INT32
            }

            let key = (def_line_no, operand.clone());
            if seen_failures.contains(&key) {
                continue;
            }
            seen_failures.insert(key);

            out.push(
                Verdict::new(Status::Fail, "K04")
                    .file_opt(vfile(file))
                    .function(fn_.name.clone())
                    .line_no(def_line_no)
                    .diagnostic(format!(
                        "`LDR W{w_digits}, [X29, #{off}]` zero-extends a {decl} slot, then `CMP {}, {}` \
at line {} feeds `B.{}` (signed). Negative {decl} values compare wrong. Bug #49-class regression.",
                        site.cmp_lhs, site.cmp_rhs, site.cmp_line_no, site.branch_suffix
                    ))
                    .suggested_fix(format!(
                        "Replace `LDR W{w_digits}, [X29, #{off}]` with `LDRSW X{w_digits}, [X29, #{off}]` \
to sign-extend the {decl} value before the signed compare."
                    )),
            );
        }
    }

    if out.is_empty() {
        out.push(
            Verdict::new(Status::Pass, "K04")
                .file_opt(vfile(file))
                .function(fn_.name.clone())
                .line_no(fn_.start_line)
                .diagnostic(format!("sign-extension before signed compare OK ({} signed-branch site(s))", sites.len())),
        );
    }
    out
}

// =====================================================================
// K05 — cache + interrupt hygiene at boot handoff
// =====================================================================

fn matches_mov_sp(code: &str) -> bool {
    (|| -> Option<()> {
        let rest = pattern::lit_req_ws(code, "MOV")?;
        let rest = pattern::lit(rest, "SP")?;
        let rest = pattern::lit(rest, ",")?;
        let (_, rest) = pattern::reg_token(rest, 'X')?;
        pattern::at_end(rest).then_some(())
    })()
    .is_some()
}

fn matches_msr_daifset(code: &str) -> bool {
    (|| -> Option<()> {
        let rest = pattern::lit_req_ws_ci(code, "MSR")?;
        let rest = pattern::lit_ci(rest, "DAIFSet")?;
        let rest = pattern::lit(rest, ",")?;
        let rest = pattern::skip_ws(rest);
        let rest = pattern::lit_ci(rest, "#0xF")?;
        rest.chars().next().map(|c| !parse_s::is_word_char(c)).unwrap_or(true).then_some(())
    })()
    .is_some()
}

fn matches_ic_iallu(code: &str) -> bool {
    (|| -> Option<()> {
        let rest = pattern::lit_req_ws_ci(code, "IC")?;
        let rest = pattern::lit_ci(rest, "IALLU")?;
        rest.chars().next().map(|c| !parse_s::is_word_char(c)).unwrap_or(true).then_some(())
    })()
    .is_some()
}

fn matches_dsb_sy(code: &str) -> bool {
    (|| -> Option<()> {
        let rest = pattern::lit_req_ws_ci(code, "DSB")?;
        let rest = pattern::lit_ci(rest, "SY")?;
        rest.chars().next().map(|c| !parse_s::is_word_char(c)).unwrap_or(true).then_some(())
    })()
    .is_some()
}

fn matches_isb(code: &str) -> bool {
    match pattern::lit_ci(code, "ISB") {
        Some(rest) => rest.chars().next().map(|c| !parse_s::is_word_char(c)).unwrap_or(true),
        None => false,
    }
}

/// `.*_(boot|init)\.s$`, case-insensitive — equivalent to a
/// case-insensitive "ends with `_boot.s` or `_init.s`" check, since the
/// leading `.*` can absorb any prefix and the rest of the pattern is a
/// fixed suffix.
fn matches_boot_filename(file: &str) -> bool {
    let lower = file.to_lowercase();
    lower.ends_with("_boot.s") || lower.ends_with("_init.s")
}

const EXPECTED_BARRIERS: [&str; 4] = ["MSR DAIFSet, #0xF", "IC IALLU", "DSB SY", "ISB"];

fn barrier_matches(idx: usize, code: &str) -> bool {
    match idx {
        0 => matches_msr_daifset(code),
        1 => matches_ic_iallu(code),
        2 => matches_dsb_sy(code),
        3 => matches_isb(code),
        _ => false,
    }
}

/// Statement (UFA Bare-Metal Canon §4 K05): boot stubs handing off from a
/// bootloader to UAGL must issue `MSR DAIFSet, #0xF`, `IC IALLU`,
/// `DSB SY`, `ISB` immediately after SP setup, in that order, before the
/// first call into kernel code. Gated by filename (`*_boot.s` /
/// `*_init.s`); NOT_APPLICABLE otherwise.
pub fn k05_cache_interrupt_hygiene(s_text: &str, ctx: &Ctx) -> Vec<Verdict> {
    let file = ctx.file.clone().unwrap_or_default();
    let file_opt = if file.is_empty() { None } else { Some(file.clone()) };

    if !matches_boot_filename(&file) {
        return vec![Verdict::new(Status::NotApplicable, "K05").file_opt(file_opt).diagnostic(
            "K05 applies only to *_boot.s / *_init.s files; this file is not a boot-protocol handoff point.",
        )];
    }

    let lines = parse_s::split_lines(s_text);
    let n = lines.len();

    let Some(sp_setup_idx) = lines.iter().position(|ln| matches_mov_sp(&ln.code)) else {
        return vec![Verdict::new(Status::NotApplicable, "K05")
            .file_opt(file_opt)
            .diagnostic("No `MOV SP, Xn` SP-setup found; K05 N/A.")];
    };

    let mut found_lines: Vec<u32> = Vec::new();
    let mut expect_idx: usize = 0;

    for ln in lines.iter().take(n).skip(sp_setup_idx + 1) {
        let code = &ln.code;
        if code.is_empty() {
            continue;
        }
        if code.starts_with("BL ") || code.starts_with("BLR ") || code.starts_with("BL\t") || code.starts_with("BLR\t")
        {
            break;
        }
        if expect_idx < EXPECTED_BARRIERS.len() {
            if barrier_matches(expect_idx, code) {
                found_lines.push(ln.no);
                expect_idx += 1;
                continue;
            }
            for later in (expect_idx + 1)..EXPECTED_BARRIERS.len() {
                if barrier_matches(later, code) {
                    return vec![Verdict::new(Status::Fail, "K05")
                        .file_opt(file_opt)
                        .line_no(ln.no)
                        .diagnostic(format!(
                            "barrier `{}` appears before `{}` (expected order: MSR DAIFSet → IC IALLU → DSB SY → ISB).",
                            EXPECTED_BARRIERS[later], EXPECTED_BARRIERS[expect_idx]
                        ))
                        .suggested_fix(
                            "Reorder the post-SP-setup barriers to MSR DAIFSet, #0xF / IC IALLU / \
DSB SY / ISB. See Bug #60 fix in verify/exynos_smoke_boot.s.",
                        )];
                }
            }
        }
    }

    if expect_idx == EXPECTED_BARRIERS.len() {
        return vec![Verdict::new(Status::Pass, "K05").file_opt(file_opt).line_no(*found_lines.last().unwrap()).diagnostic(
            format!(
                "Bug #60 four-instruction barrier sequence present after SP setup (lines {}-{}).",
                found_lines[0],
                found_lines.last().unwrap()
            ),
        )];
    }

    let missing: Vec<&str> = EXPECTED_BARRIERS[expect_idx..].to_vec();
    vec![Verdict::new(Status::Fail, "K05")
        .file_opt(file_opt)
        .line_no(lines[sp_setup_idx].no)
        .diagnostic(format!(
            "missing post-SP-setup barriers: {}. Bug #60 (cache + interrupt hygiene at boot handoff) \
regression — kernel may execute on a stale I-cache or with interrupts unmasked.",
            missing.join(", ")
        ))
        .suggested_fix(
            // NOTE: the 4-space indent on the MSR/IC/DSB/ISB lines is
            // placed at the END of the *preceding* line, right before the
            // continuation backslash, not at the start of its own line.
            // Rust's `\<newline>` string continuation strips ALL leading
            // whitespace on the line it joins to (not just the newline) --
            // indenting the continued lines themselves (as this block used
            // to) silently loses that indentation from the actual string,
            // even though it's still visible in the source. This is not
            // cosmetic: `tests/golden.rs`'s
            // k05_catches_missing_barriers_in_boot_file pins the indented
            // form, captured verbatim from the Python original's CLI output.
            "Insert the four-instruction sequence immediately after `MOV SP, Xn`:\n    \
MSR DAIFSet, #0xF      // mask SError, IRQ, FIQ, debug\n    \
IC  IALLU              // invalidate entire local I-cache\n    \
DSB SY                 // wait for cache op to drain\n    \
ISB                    // flush pipeline so next fetch is clean\n\
See verify/exynos_smoke_boot.s for the canonical post-Bug-#60 form.",
        )]
}

// =====================================================================
// K06 — section & alignment discipline for data globals
// =====================================================================

const DIR_ALIGN_FLOOR: &[(&str, u32)] = &[
    ("byte", 0),
    ("asciz", 0),
    ("ascii", 0),
    ("string", 0),
    ("short", 1),
    ("hword", 1),
    ("word", 2),
    ("long", 2),
    ("quad", 3),
];

fn dir_align_floor(d: &str) -> u32 {
    DIR_ALIGN_FLOOR.iter().find(|(k, _)| *k == d).map(|(_, v)| *v).unwrap_or(3)
}

/// Largest log2 alignment required by any element directive in `g`'s
/// payload. The `payload.is_empty()` default of 3 is unreachable from
/// `k06_check_global`'s only call site (`array_const` implies non-empty
/// payload per `classify_data_global`), ported anyway for fidelity.
fn max_payload_align_floor(g: &DataGlobal) -> u32 {
    if g.payload.is_empty() {
        return 3;
    }
    g.payload.iter().map(|(_, d, _, _)| dir_align_floor(d)).max().unwrap_or(3)
}

/// Statement (UFA Bare-Metal Canon §4 K06): const-init arrays and string
/// literals go to `.rodata`; zero-init buffers go to `.bss`/`.data`; every
/// global is at least the alignment its element width and const-ness
/// require (byte-arrays: 1B; scalars/zero-init: 8B; arrays: per widest
/// element directive).
pub fn k06_section_alignment(s_text: &str, ctx: &Ctx) -> Vec<Verdict> {
    let file = ctx.file.as_deref();
    let mut verdicts = Vec::new();
    let globals_ = parse_s::parse_globals(s_text);

    if !globals_.iter().any(|g| !g.is_text_section) {
        verdicts.push(
            Verdict::new(Status::NotApplicable, "K06")
                .file_opt(vfile(file))
                .diagnostic("No data globals in this file; K06 N/A."),
        );
        return verdicts;
    }

    for g in &globals_ {
        if g.is_text_section {
            continue; // function entry -- defer to ISA / assembler
        }
        verdicts.extend(k06_check_global(g, file));
    }

    verdicts
}

fn k06_check_global(g: &DataGlobal, file: Option<&str>) -> Vec<Verdict> {
    let mut out: Vec<Verdict> = Vec::new();
    let cls = parse_s::classify_data_global(g);
    let is_byte = parse_s::is_byte_array_global(g);

    let required_align: u32 = if is_byte {
        0
    } else if matches!(cls, "scalar_const" | "empty") {
        3
    } else if cls == "zero_init" {
        3
    } else if cls == "array_const" {
        max_payload_align_floor(g)
    } else {
        3
    };

    match g.align_log2 {
        None => {
            out.push(
                Verdict::new(Status::Fail, "K06")
                    .file_opt(vfile(file))
                    .function(g.name.clone())
                    .line_no(g.label_line)
                    .diagnostic(format!(
                        "global `{}` (class={cls}) has no explicit `.align` directive; emitter must align all data globals.",
                        g.name
                    ))
                    .suggested_fix(format!(
                        "Emit `.align {required_align}` before the `{}:` label. See Bug #18 / Bug #37.",
                        g.name
                    )),
            );
        }
        Some(align_log2) if align_log2 < required_align => {
            out.push(
                Verdict::new(Status::Fail, "K06")
                    .file_opt(vfile(file))
                    .function(g.name.clone())
                    .line_no(g.align_line.unwrap_or(g.label_line))
                    .diagnostic(format!(
                        "global `{}` (class={cls}) has `.align {align_log2}` ({}-byte); requires \
`.align {required_align}` ({}-byte) at minimum. Bug #18 / Bug #37 regression.",
                        g.name,
                        1u64 << align_log2,
                        1u64 << required_align
                    ))
                    .suggested_fix(format!(
                        "Bump alignment to `.align {required_align}` ({}-byte). For scalars whose \
value fits in 4 bytes but get loaded via `LDR X`, `.align 3` is required to avoid alignment fault.",
                        1u64 << required_align
                    )),
            );
        }
        _ => {}
    }

    // .rodata routing for const-init arrays and strings (Bug #55).
    if cls == "array_const" && !g.section.starts_with(".rodata") {
        out.push(
            Verdict::new(Status::Fail, "K06")
                .file_opt(vfile(file))
                .function(g.name.clone())
                .line_no(g.label_line)
                .diagnostic(format!(
                    "const-init array `{}` lives in `{}` (writable). Bug #55 requires const arrays \
/ string literals to land in `.rodata` so a stray write traps.",
                    g.name, g.section
                ))
                .suggested_fix(
                    "Update `_emit_file_footer` to route globals with concrete `[v1, v2, ...]` or \
`\"text\"` initializers into `.rodata` (R-X segment).",
                ),
        );
    }

    if out.is_empty() {
        let align_str = g.align_log2.map(|a| a.to_string()).unwrap_or_else(|| "None".to_string());
        out.push(
            Verdict::new(Status::Pass, "K06")
                .file_opt(vfile(file))
                .function(g.name.clone())
                .line_no(g.label_line)
                .diagnostic(format!("`{}` ({cls}) at `.align {align_str}` in `{}` OK", g.name, g.section)),
        );
    }
    out
}

// =====================================================================
// Registry (single source of truth)
// =====================================================================

pub type CheckFn = fn(&str, &Ctx) -> Vec<Verdict>;

/// Mirrors `K_REGISTRY` in `k_invariants.py`. A `&'static [(&str, CheckFn)]`
/// preserves definition order the way Python's insertion-ordered `dict`
/// does (`codegen_audit.rs`'s determinism guarantee depends on this),
/// which a `HashMap` would not.
pub const K_REGISTRY: &[(&str, CheckFn)] = &[
    ("K01", k01_stack_frame_discipline),
    ("K02", k02_register_spill_balance),
    ("K03", k03_type_width_load_store),
    ("K04", k04_sign_extension),
    ("K05", k05_cache_interrupt_hygiene),
    ("K06", k06_section_alignment),
];

/// Looks up a single check by name (used by the CLI's `--k` filter and
/// the GUI's per-check toggles).
pub fn get(name: &str) -> Option<CheckFn> {
    K_REGISTRY.iter().find(|(n, _)| *n == name).map(|(_, f)| *f)
}

/// All registered K-class names, in registry (definition) order.
pub fn names() -> Vec<&'static str> {
    K_REGISTRY.iter().map(|(n, _)| *n).collect()
}
