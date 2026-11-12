//! Port of `regression_map.py`.
//!
//! Bug → K-class mapping, copied verbatim from
//! `docs/UFA_BAREMETAL_CANON_BRIEF.md` v2 §4.5.
//!
//! This module is NOT the place to reclassify bugs. The §4.5 table is the
//! contract for the entire factory; if a mapping looks wrong, surface it,
//! don't silently amend it here — per brief §6 Step 1.7, the same
//! discipline the Python original states explicitly. The table below is a
//! line-for-line transcription of `BUG_TO_K` (including its section
//! comments), re-read fresh from the source before transcribing rather
//! than reconstructed from memory.
//!
//! Sentinel values (not K-class strings): [`NA`] (not invariant-bearing),
//! [`DEFER`] (deferred per the source's own status), [`SKIP`] (numbering
//! gap), and the literal `"K03/K04"` (symptom-only catch across two
//! K-classes; root cause out of scope per brief §8).

use std::collections::BTreeMap;

pub const NA: &str = "N/A";
pub const DEFER: &str = "DEFER";
pub const SKIP: &str = "SKIP";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BugMapping {
    pub bug_id: &'static str,
    pub summary: &'static str,
    pub k_class: &'static str,
    pub reason: &'static str,
    /// One of `"covered"` (the K-class implementation actually catches
    /// this bug shape), `"deferred"` (the §4.5 column maps this bug to a
    /// K-class, but the K-check implementation explicitly defers this
    /// specific shape to a future Kxx v2 — Council audit 2026-05-06
    /// finding), or `"n/a"` (k_class is a sentinel; detection doesn't
    /// apply).
    pub detection: &'static str,
}

const fn bm(
    bug_id: &'static str,
    summary: &'static str,
    k_class: &'static str,
    reason: &'static str,
    detection: &'static str,
) -> BugMapping {
    BugMapping { bug_id, summary, k_class, reason, detection }
}

// ── BUG_TO_K — verbatim from brief §4.5 (v2, 2026-05-06) ────────────────
//
// Counts per §4.5: K01:8, K02:4, K03:7, K04:2, K05:1, K06:7.
// Plus N/A: 22. DEFER: 2. SKIP: 2 (#13, #14 numbering gaps). Symptom-only
// (root cause out of scope per §8): 4 — recorded as "K03/K04".
//
// DO NOT REORDER. DO NOT RECLASSIFY. Source: docs/UFA_BAREMETAL_CANON_BRIEF.md
// §4.5. If you disagree with a mapping, STOP and surface to the user.
pub const BUG_TO_K: &[BugMapping] = &[
    bm("1", "Stack double-allocation (per-decl SUB SP)", "K01", "Frame discipline", "covered"),
    bm("2", "Local var resolved as global (ADRP)", NA, "Compiler routing feature, not invariant violation", "covered"),
    bm("3", "* operator MUL → INT32 needs SMULL", "K03", "Type-width on multiply", "covered"),
    bm("4", "Increment ─ ─ shorthand", NA, "New syntax feature", "covered"),
    bm("5", "Array subscript arr[idx] basic compile", NA, "Feature compile-passed, no closed invariant", "covered"),
    bm("6", "Float literal 0.15 ADRP", DEFER, "Bug #6 deferred per source", "covered"),
    bm("7", "Struct field assign basic compile", NA, "Feature compile-passed", "covered"),
    bm("8", "QBX linker offset 0 export", DEFER, "Out of critical path", "covered"),
    bm("9", "Inline trailing /* */ lexer", NA, "Lexer feature", "covered"),
    bm("10", "Multi-line block comment lexer", NA, "Lexer feature", "covered"),
    // Council 2026-05-06 (H&P, Knuth, Lyubashevsky): the §4.5 column maps
    // #11 -> K04, but k_invariants.py K04 v1 explicitly defers the
    // ASR/LSR shift-signedness check (see K04 docstring "Deferred K04
    // v2"). Detection requires per-shift operand-type tracking similar
    // to K03's MUL state machine; out of v1 scope. Surfaced as
    // detection="deferred" so verify_bug_coverage() does not silently
    // over-claim.
    bm("11", "Shift << >> operators", "K04", "ASR/LSR signedness", "deferred"),
    bm("12", "Nested binop register clobbering", "K02", "X9 spill/restore", "covered"),
    bm("13", "(skipped — numbering gap)", SKIP, "Numbering gap; no recorded bug", "covered"),
    bm("14", "(skipped — numbering gap)", SKIP, "Numbering gap; no recorded bug", "covered"),
    bm("15", "Trailing newline missing", NA, "Cosmetic, non-invariant", "covered"),
    bm("16", "Linker RWX permissions warning", NA, "Linker config, cross-file (out of K06 scope per §8)", "covered"),
    bm("17", "Top-level · name GLOBAL routing", NA, "Parser feature", "covered"),
    bm("18", "4-byte global Data Abort on LDR X", "K06", "Alignment (also K03 width-aware)", "covered"),
    bm("19", "SUB SP, SP, #6784 immediate range", "K01", "Frame size encoding", "covered"),
    bm("20", "Local stack offset off-by-one", "K01", "Frame size accounting", "covered"),
    bm("21", "SP-rel locals push/pop bozuluyor", "K01", "Frame discipline (FP fix)", "covered"),
    // Council 2026-05-06 #2: K02 v1 implements scratch-register spill
    // balance only. Caller-save discipline across BL (X0-X18 clobber
    // tracking) is K02 v2 -- needs forward register-liveness analysis.
    bm("22", "Param register clobber across BL", "K02", "Caller/callee-save discipline", "deferred"),
    bm("23", "CMP LHS clobber by RHS", "K02", "Spill before RHS load", "covered"),
    bm("24", "Multi-line array global lex", NA, "Lexer feature", "covered"),
    bm("25", "parse_params @StructName", NA, "Parser feature", "covered"),
    bm("26", "type_size crash on unsized array", NA, "Compiler-internal hardening", "covered"),
    bm("27", "Struct field stride hardcoded INT32", "K03", "Per-field type-aware stride", "covered"),
    bm("28a", "Global array element-type defaults INT32", "K03", "Type-width on global stride", "covered"),
    // K01 v1 catches frame-balance + STR XZR alignment but does NOT
    // cross-reference the emitted zero-block byte-count against the
    // local's declared T[N] size (no source map). K01 v2 expansion.
    bm("28b", "Local T[N] = 0 first element only", "K01", "Zero-block coverage", "deferred"),
    bm("29", ".quad everywhere → per-type directive", "K03", "Section directive width", "covered"),
    bm("30", "_operand_aarch64_type blind", "K03/K04", "Symptom-only; root cause out of scope per §8", "covered"),
    bm("31", "Standalone ▽ else block fall-through", NA, "Codegen feature", "covered"),
    bm("32", "(same as #28b)", SKIP, "Already mapped under #28b", "covered"),
    // Same as #28b: K01 v1 does not validate emitted zero-block size
    // against declared local size. K01 v2 expansion.
    bm("33", "Inline-struct local zero-init only 8B", "K01", "Zero-block coverage", "deferred"),
    bm("34", "No struct field offset table", NA, "Compiler-internal feature", "covered"),
    // Council 2026-05-06 #2: K02 v1 does not track AAPCS64 callee-save
    // registers (X19-X28). K02 v2 -- needs per-function "saved-register
    // set" parse from the prologue and a use-of-each-register check in
    // the body.
    bm("35", "uart_print_hex64 clobbered X20", "K02", "Callee-save in helper asm", "deferred"),
    bm("36", "&block[i*8] for pointer param", NA, "Codegen feature", "covered"),
    bm("37", "U32 scalar .align 2 vs .quad", "K06", "Alignment + section", "covered"),
    bm("38", "_emit_struct_base_addr inline-struct", NA, "Codegen feature", "covered"),
    bm("39", "Top-level struct global .quad 0 truncate", "K06", "Size in section directive", "covered"),
    bm("40", "polyvecl_pointwise_acc_montgomery zero", NA, "Downstream of #41 fix", "covered"),
    bm("41", "state.type_of global lookup", "K03/K04", "Symptom-only; root cause out of scope per §8", "covered"),
    bm("42", "_emit_struct_base_addr non-pointer global", NA, "Codegen feature; symptom in K03 if mis-emitted", "covered"),
    bm("43", "Struct elem field LDR LSL >3", "K03", "Load instruction LSL bound", "covered"),
    bm("44", "_operand_aarch64_type UINT64 fallback (KRİTİK)", "K03/K04", "Symptom-only; root cause out of scope per §8", "covered"),
    bm("45", "crypto_sign rejection loop never accepts", NA, "UAGL source bug, fixed via #47/#48/#49", "covered"),
    bm("46", "crypto_verify Python ref REJECT", NA, "Downstream / UAGL source", "covered"),
    // K06 docstring "Deferred (K06 v2): Bug #47 -- U8[N] over-allocation".
    // Detection requires UAGL-source declared length (only in source
    // comments). K06 v1 catches alignment + section routing for byte
    // arrays; the over-allocation invariant needs the cross-reference.
    bm("47", "U8[N] over-allocate N+7", "K06", "Sizing", "deferred"),
    bm("48", "Polyveck/Polyvecl global .quad 0 truncate", "K06", "Struct size in section", "covered"),
    bm("49", "INT32/S32 LDR X stale upper bits", "K04", "Sign-extension via LDRSW", "covered"),
    bm("50", "UAGL source NTT in-place", NA, "UAGL source bug, out of K-checker scope", "covered"),
    bm("51", "U8[N] = \"string\" literal init", "K06", "Section placement (also Stable Invariant #5)", "covered"),
    bm("52", "mem[addr] MMIO codegen", NA, "New feature (also Stable Invariant #4 — distinct opcode shape, K01 covers)", "covered"),
    bm("53", "_aarch64_addsub_imm_lines(0) returned []", "K01", "Frame helper correctness", "covered"),
    bm("54", "STR XZR unaligned hangs cortex-a53", "K01", "Stable Invariant #3", "covered"),
    bm("55", "Const tables to .rodata", "K06", "Stable Invariant #6", "covered"),
    bm("56", "__memcpy / __memset intrinsics", NA, "New feature", "covered"),
    bm("57", "__asm__ raw asm", NA, "New feature", "covered"),
    bm("58", "Function pointer indirect call", NA, "New feature (also Stable Invariant: BLR not BL)", "covered"),
    bm("59", "Tuple-return X0+X1 pair", NA, "New feature (also Stable Invariant #8 — K02 covers receive ordering)", "covered"),
    bm("60", "Cache hygiene at handoff", "K05", "Boot-path barriers", "covered"),
    bm("61", "Array `{...}` initializer accepted in single-line global decl", NA, "Compiler parser fix (caught while debugging AES_RCON zero-emit)", "covered"),
    // Bug #62 entry: factory's first earned dividend. Caught by K05 on
    // the first sweep over verify/*.s files; sister fix to Bug #60.
    // (Bug #63 = the UFA Bare-Metal Canon factory itself; it isn't
    //  detectable BY itself, so no K-class -- listed for traceability.)
    bm("62", "tuple_test_boot.s missing handoff barriers (caught by K-check factory)", "K05", "Boot-path barriers", "covered"),
    bm("63", "UFA Bare-Metal Canon factory itself (this checker)", NA, "Self-referential — the factory cannot K-check itself; tracked for catalog completeness", "covered"),
    bm(
        "64",
        "GHASH constant-time variant (mask-AND replaces secret-dep branch) + AAD-bearing decrypt + CONSTANT_TIME_AUDIT.md",
        NA,
        "Constant-time refinement; sits outside K01-K06 scope per Council #8 — covered by docs/CONSTANT_TIME_AUDIT.md instead",
        "covered",
    ),
];

// ── Coverage verification ───────────────────────────────────────────────

const VALID_K_CLASSES: &[&str] = &["K01", "K02", "K03", "K04", "K05", "K06"];
const VALID_SENTINELS: &[&str] = &[NA, DEFER, SKIP, "K03/K04"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageReport {
    pub by_k: BTreeMap<&'static str, Vec<&'static str>>,
    pub counts: BTreeMap<&'static str, usize>,
    pub deferred_by_k: BTreeMap<&'static str, Vec<&'static str>>,
}

/// Mirrors `verify_bug_coverage()` in `regression_map.py`. Walks the table
/// and asserts every entry is well-formed (valid k_class, non-empty
/// reason/summary, valid detection value for real K-class rows). Returns
/// `Err` with the same joined-message shape as the Python
/// `AssertionError` instead of panicking — this port treats malformed
/// data as a recoverable `Result`, not a crash, so a caller (CLI or GUI)
/// can display it.
pub fn verify_bug_coverage(table: &[BugMapping]) -> Result<CoverageReport, String> {
    let mut by_k: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut deferred_by_k: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
    let mut invalid: Vec<String> = Vec::new();

    for m in table {
        let k = m.k_class;
        if !VALID_K_CLASSES.contains(&k) && !VALID_SENTINELS.contains(&k) {
            invalid.push(format!("#{}: unknown k_class {:?}", m.bug_id, k));
            continue;
        }
        if m.reason.trim().is_empty() {
            invalid.push(format!("#{}: empty reason field (every entry must have a one-line reason)", m.bug_id));
            continue;
        }
        if m.summary.trim().is_empty() {
            invalid.push(format!("#{}: empty summary field", m.bug_id));
            continue;
        }
        if VALID_K_CLASSES.contains(&k) && m.detection != "covered" && m.detection != "deferred" {
            invalid.push(format!(
                "#{}: detection={:?} for K-class row must be 'covered' or 'deferred'",
                m.bug_id, m.detection
            ));
            continue;
        }
        by_k.entry(k).or_default().push(m.bug_id);
        *counts.entry(k).or_insert(0) += 1;
        if m.detection == "deferred" && VALID_K_CLASSES.contains(&k) {
            deferred_by_k.entry(k).or_default().push(m.bug_id);
        }
    }

    if !invalid.is_empty() {
        return Err(format!("BUG_TO_K coverage failed:\n  {}", invalid.join("\n  ")));
    }

    Ok(CoverageReport { by_k, counts, deferred_by_k })
}

/// Mirrors `k_class_for_bug()` in `regression_map.py`.
pub fn k_class_for_bug(bug_id: &str) -> Option<&'static str> {
    BUG_TO_K.iter().find(|m| m.bug_id == bug_id).map(|m| m.k_class)
}

/// Sort #28a < #28b < #29 (numeric primary, suffix secondary). Mirrors
/// `_sort_key()`: filters ALL digit characters out of the id (not just a
/// leading run) and slices the remainder by that count. This only
/// behaves "as a prefix split" for ids shaped like ours (digits, then an
/// optional letter suffix) — which is every id in [`BUG_TO_K`] — so the
/// direct translation is safe for the actual data even though it would
/// be an odd algorithm for an arbitrary string.
fn sort_key(bug_id: &str) -> (u32, String) {
    let digit_count = bug_id.chars().filter(|c| c.is_ascii_digit()).count();
    let digits: String = bug_id.chars().filter(|c| c.is_ascii_digit()).collect();
    let suffix: String = bug_id.chars().skip(digit_count).collect();
    (digits.parse().unwrap_or(0), suffix)
}

const EXPECTED_COUNTS: &[(&str, usize)] =
    &[("K01", 8), ("K02", 4), ("K03", 5), ("K04", 2), ("K05", 2), ("K06", 7)];

/// Mirrors `_main()`'s report body in `regression_map.py` (the CLI's
/// `bug-coverage` subcommand calls this; the GUI's coverage panel renders
/// the same text via [`coverage_report_string`]).
///
/// Expected counts are the column-literal counts from brief §4.5, adjusted
/// in two places per the Python docstring: K05=2 (Bug #62 added in Step 5
/// as the factory's first earned dividend, vs. the brief's original
/// Bug-#60-only count of 1), and K03=5 kept column-verbatim rather than
/// the brief's bottom-note union count of 7 (which folds in #18's and
/// #44's also-K03 cross-references) per the "do not reclassify" instruction.
pub fn print_coverage_report(out: &mut impl std::io::Write) -> std::io::Result<()> {
    let report = verify_bug_coverage(BUG_TO_K).expect("bundled BUG_TO_K table is well-formed");

    writeln!(out, "BUG_TO_K coverage report (verbatim from brief §4.5)")?;
    writeln!(out, "{}", "=".repeat(64))?;

    for &(k, exp) in EXPECTED_COUNTS {
        let actual = *report.counts.get(k).unwrap_or(&0);
        let marker = if actual == exp { "OK ".to_string() } else { format!("!! (expected {exp})") };
        let mut ids: Vec<&str> = report.by_k.get(k).cloned().unwrap_or_default();
        ids.sort_by_key(|id| sort_key(id));
        let no_deferred: Vec<&str> = Vec::new();
        let deferred = report.deferred_by_k.get(k).unwrap_or(&no_deferred);
        let ids_render: Vec<String> =
            ids.iter().map(|id| if deferred.contains(id) { format!("{id}†") } else { id.to_string() }).collect();
        let covered = ids.iter().filter(|id| !deferred.contains(id)).count();
        let cover_marker =
            if deferred.is_empty() { "(all covered)".to_string() } else { format!("({covered}/{actual} covered)") };
        writeln!(out, "  {k}: {actual} {marker:<6}  {cover_marker:<18} [{}]", ids_render.join(", "))?;
    }

    for sentinel in ["K03/K04", NA, DEFER, SKIP] {
        let n = *report.counts.get(sentinel).unwrap_or(&0);
        let mut ids: Vec<&str> = report.by_k.get(sentinel).cloned().unwrap_or_default();
        ids.sort_by_key(|id| sort_key(id));
        writeln!(out, "  {sentinel:<9} {n:>3}                              [{}]", ids.join(", "))?;
    }

    let total: usize = report.counts.values().sum();
    writeln!(out, "  total   {total:>3}")?;
    writeln!(out)?;
    writeln!(out, "† = deferred detection — k_class is mapped per §4.5 column, but")?;
    writeln!(out, "    the K-check implementation defers the specific shape to a")?;
    writeln!(out, "    future Kxx v2 expansion. See ufa_baremetal_core::regression_map.")?;

    if let Some(b62) = BUG_TO_K.iter().find(|m| m.bug_id == "62") {
        writeln!(out)?;
        writeln!(out, "Note: #62 '{}' → {} (factory dividend).", b62.summary, b62.k_class)?;
    }

    Ok(())
}

/// Convenience wrapper for callers (the GUI) that want the report as a
/// `String` rather than writing to an `io::Write`.
pub fn coverage_report_string() -> String {
    let mut buf: Vec<u8> = Vec::new();
    print_coverage_report(&mut buf).expect("writing to an in-memory buffer cannot fail");
    String::from_utf8(buf).expect("report text is constructed from ASCII/UTF-8 literals")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_65_entries_matching_python_original() {
        assert_eq!(BUG_TO_K.len(), 65);
    }

    #[test]
    fn bundled_table_passes_its_own_coverage_check() {
        let report = verify_bug_coverage(BUG_TO_K).expect("bundled table must be well-formed");
        assert_eq!(report.counts.get("K01"), Some(&8));
        assert_eq!(report.counts.get("K02"), Some(&4));
        assert_eq!(report.counts.get("K03"), Some(&5));
        assert_eq!(report.counts.get("K04"), Some(&2));
        assert_eq!(report.counts.get("K05"), Some(&2));
        assert_eq!(report.counts.get("K06"), Some(&7));
        assert_eq!(report.counts.get("SKIP"), Some(&3));
        let total: usize = report.counts.values().sum();
        assert_eq!(total, 65);
    }

    #[test]
    fn k_class_lookup() {
        assert_eq!(k_class_for_bug("54"), Some("K01"));
        assert_eq!(k_class_for_bug("60"), Some("K05"));
        assert_eq!(k_class_for_bug("no-such-bug"), None);
    }

    #[test]
    fn sort_key_orders_lettered_suffixes_after_bare_number() {
        let mut ids = vec!["29", "28b", "28a", "3"];
        ids.sort_by_key(|id| sort_key(id));
        assert_eq!(ids, vec!["3", "28a", "28b", "29"]);
    }
}
