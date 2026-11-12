//! Port of `codegen_audit.py`.
//!
//! Orchestrates every K-check in [`crate::k_invariants::K_REGISTRY`]
//! across one `.s` file or one directory of `.s` files. Pure functions
//! over text + path; no process exit codes here (the CLI translates
//! verdicts into one — see `bin/ufa-baremetal`).
//!
//! Determinism: [`crate::k_invariants::K_REGISTRY`] is iterated in
//! definition order (a `&'static [(&str, CheckFn)]`, not a hash map).
//! [`audit_directory`] walks paths in sorted order. Output for the same
//! input set is byte-stable across machines.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::k_invariants::{self, Ctx};
use crate::parse_s::{self, ParseCompletenessError};
use crate::verdict::{Status, Verdict};

#[derive(Debug)]
pub enum AuditError {
    Io(PathBuf, std::io::Error),
    NotADirectory(PathBuf),
    ParseCompleteness(ParseCompletenessError),
}

impl std::fmt::Display for AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            AuditError::NotADirectory(path) => write!(f, "{} is not a directory", path.display()),
            AuditError::ParseCompleteness(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AuditError {}

/// The filesystem-free core of `audit_file()` in `codegen_audit.py`: runs
/// every check in [`k_invariants::K_REGISTRY`] over `s_text` that's
/// already in memory -- no `Path`, no `fs::read`. This is what makes an
/// in-browser (WASM) or in-editor (future LSP, #4 on the roadmap) caller
/// possible at all: neither has a real filesystem path for an unsaved
/// buffer or a pasted snippet, only text.
///
/// Two separate name parameters because [`audit_file`] itself needs two
/// different strings here: a full path for `ParseCompletenessError`'s
/// message (`parse_display_name`, more useful for locating the file when
/// something's wrong), and just a basename for `Verdict.file` (`ctx_file`,
/// shorter, and what the existing golden tests pin). Most callers without
/// a real filesystem path -- a pasted snippet, an LSP document URI --
/// don't have that distinction to make, which is what [`audit_text`] is
/// for.
///
/// Pre-flight: [`parse_s::assert_parse_complete`] refuses to run K-checks
/// if `parse_functions` silently dropped any `.type X, %function`
/// declaration (Council audit 2026-05-06 finding). Such a parse gap
/// surfaces as `Err`, which callers should treat as build-blocking (the
/// CLI maps [`AuditError::ParseCompleteness`], which wraps this same
/// error, to exit 2). K-check verdicts on a partial parse would be
/// vacuous PASSes that mask real bugs.
pub fn audit_text_named(
    s_text: &str,
    parse_display_name: &str,
    ctx_file: &str,
) -> Result<Vec<Verdict>, ParseCompletenessError> {
    audit_text_with_mask(s_text, parse_display_name, ctx_file, u32::MAX)
}

fn audit_text_with_mask(
    s_text: &str,
    parse_display_name: &str,
    ctx_file: &str,
    mask: u32,
) -> Result<Vec<Verdict>, ParseCompletenessError> {
    parse_s::assert_parse_complete(s_text, parse_display_name)?;
    let ctx = Ctx::with_file(ctx_file);
    let mut out = Vec::new();
    for (index, (_, check)) in k_invariants::K_REGISTRY.iter().enumerate() {
        if mask & (1 << index) != 0 {
            out.extend(check(s_text, &ctx));
        }
    }
    Ok(out)
}

/// Convenience wrapper over [`audit_text_named`] for the common case of
/// one name, not two -- a pasted `.s` snippet, an in-memory buffer keyed
/// by a single logical name, anything without a real filesystem path to
/// separately report in a parse-completeness error.
pub fn audit_text(s_text: &str, name: &str) -> Result<Vec<Verdict>, ParseCompletenessError> {
    audit_text_named(s_text, name, name)
}

/// Run selected checks in registry order. Bit zero selects K01, bit one
/// selects K02, and so on. The parse completeness check always runs first.
pub fn audit_text_selected(
    s_text: &str,
    name: &str,
    mask: u32,
) -> Result<Vec<Verdict>, ParseCompletenessError> {
    audit_text_with_mask(s_text, name, name, mask)
}

/// Mirrors `audit_file()` in `codegen_audit.py`.
pub fn audit_file(path: &Path) -> Result<Vec<Verdict>, AuditError> {
    let text = fs::read_to_string(path).map_err(|e| AuditError::Io(path.to_path_buf(), e))?;
    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    audit_text_named(&text, &path.display().to_string(), filename).map_err(AuditError::ParseCompleteness)
}

/// Supports only the one shape this codebase actually uses (`"*.s"`): a
/// single leading `*` wildcard matched as a suffix. Anything else falls
/// back to an exact-name match. Python's `Path.glob` is more general;
/// nothing here calls this with a richer pattern.
fn matches_simple_glob(name: &str, pattern: &str) -> bool {
    match pattern.strip_prefix('*') {
        Some(suffix) => name.ends_with(suffix),
        None => name == pattern,
    }
}

/// Mirrors `audit_directory()` in `codegen_audit.py`: runs every K-check
/// against every `.s` file directly under `path` (non-recursive), keyed
/// by filename, with files visited in sorted order so diagnostics are
/// stable across runs.
///
/// Checks each file on a small pool of worker threads (`std::thread`
/// only — no new dependency) rather than sequentially. This is safe
/// because every K-check is a pure function of one file's text: no
/// shared mutable state, no cross-file ordering requirement, so there is
/// nothing for concurrent workers to race on. Determinism is preserved
/// regardless of thread scheduling: results are tagged with each file's
/// position in the sorted file list and re-sorted back into that order
/// before being folded into the returned `BTreeMap` (whose own key order
/// is alphabetical regardless), so the output is identical to the
/// sequential version, just produced faster on multi-core machines.
///
/// One behavioral note versus a naive sequential loop: on error, the
/// sequential version stops at the first failing file in sorted order
/// and never touches the rest. Since work here is already dispatched in
/// parallel, this version lets every in-flight check finish and then
/// surfaces the *same* first-by-sorted-order error via `?` — the
/// propagated error is identical either way, this version just doesn't
/// (cheaply) avoid the wasted work of finishing already-dispatched
/// checks past the first failure. Given `AuditError` should be rare in
/// practice (a malformed file, not a normal K-check FAIL), that's a
/// reasonable trade.
pub fn audit_directory(path: &Path, glob: &str) -> Result<BTreeMap<String, Vec<Verdict>>, AuditError> {
    if !path.is_dir() {
        return Err(AuditError::NotADirectory(path.to_path_buf()));
    }
    let mut files: Vec<PathBuf> = fs::read_dir(path)
        .map_err(|e| AuditError::Io(path.to_path_buf(), e))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|s| s.to_str())
                    .map(|name| matches_simple_glob(name, glob))
                    .unwrap_or(false)
        })
        .collect();
    files.sort();

    if files.is_empty() {
        return Ok(BTreeMap::new());
    }

    let n_workers = worker_count(files.len());
    let per_file: Vec<(usize, Result<(String, Vec<Verdict>), AuditError>)> = if n_workers <= 1 {
        // Small job, or a single-core box: skip the threading machinery
        // entirely rather than pay for it with nothing to gain.
        files.iter().enumerate().map(|(i, f)| (i, audit_one(f))).collect()
    } else {
        audit_files_parallel(&files, n_workers)
    };

    let mut ordered = per_file;
    ordered.sort_by_key(|(idx, _)| *idx);

    let mut out = BTreeMap::new();
    for (_, result) in ordered {
        let (name, verdicts) = result?;
        out.insert(name, verdicts);
    }
    Ok(out)
}

fn audit_one(f: &Path) -> Result<(String, Vec<Verdict>), AuditError> {
    let name = f.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    audit_file(f).map(|verdicts| (name, verdicts))
}

/// How many worker threads to use for `files_len` independent jobs:
/// never more than the available hardware parallelism, never more than
/// one per job, and always at least one.
fn worker_count(files_len: usize) -> usize {
    let available = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    available.min(files_len).max(1)
}

/// Work-stealing fan-out over `files`: each of `n_workers` threads pulls
/// the next unclaimed index from a shared atomic cursor until the list is
/// exhausted. This balances load across threads even when files vary
/// enough in size/complexity that a naive fixed up-front split (e.g.
/// "thread 0 gets the first third") would leave some threads idle while
/// others are still working through larger files.
fn audit_files_parallel(files: &[PathBuf], n_workers: usize) -> Vec<(usize, Result<(String, Vec<Verdict>), AuditError>)> {
    let next_index = std::sync::atomic::AtomicUsize::new(0);

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..n_workers)
            .map(|_| {
                let next_index = &next_index;
                scope.spawn(move || {
                    let mut local = Vec::new();
                    loop {
                        let idx = next_index.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if idx >= files.len() {
                            break;
                        }
                        local.push((idx, audit_one(&files[idx])));
                    }
                    local
                })
            })
            .collect();

        handles
            .into_iter()
            // `.join()` returns `Result<T, Box<dyn Any + Send>>` on a
            // worker panic. `resume_unwind` re-raises that exact payload
            // in this thread rather than `.unwrap()`/`.expect()`, which
            // would additionally require `Box<dyn Any + Send>: Debug` --
            // avoided here rather than assumed.
            .flat_map(|h| h.join().unwrap_or_else(|payload| std::panic::resume_unwind(payload)))
            .collect()
    })
}

/// Mirrors `render_verdicts()` in `codegen_audit.py`. By default shows
/// only FAILs (the actionable output); pass `show_pass = true` to include
/// PASS / NOT_APPLICABLE lines for inspection.
pub fn render_verdicts(verdicts: &[Verdict], show_pass: bool) -> String {
    let mut lines = Vec::new();
    for v in verdicts {
        if v.is_fail() || (show_pass && v.is_pass()) {
            lines.push(v.render());
        } else if v.status == Status::NotApplicable && show_pass {
            lines.push(v.render());
        }
    }
    lines.join("\n")
}

/// Mirrors `summarize()` in `codegen_audit.py`. A fixed three-field
/// struct rather than a `HashMap<Status, usize>` — there are always
/// exactly three possible statuses, so this reads and prints more
/// directly than a map lookup at every call site.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub pass: usize,
    pub fail: usize,
    pub not_applicable: usize,
}

impl Summary {
    pub fn total(&self) -> usize {
        self.pass + self.fail + self.not_applicable
    }
}

pub fn summarize(verdicts: &[Verdict]) -> Summary {
    let mut s = Summary::default();
    for v in verdicts {
        match v.status {
            Status::Pass => s.pass += 1,
            Status::Fail => s.fail += 1,
            Status::NotApplicable => s.not_applicable += 1,
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::Verdict;

    #[test]
    fn summarize_counts_each_status() {
        let verdicts = vec![
            Verdict::new(Status::Pass, "K01"),
            Verdict::new(Status::Pass, "K02"),
            Verdict::new(Status::Fail, "K03"),
            Verdict::new(Status::NotApplicable, "K05"),
        ];
        let s = summarize(&verdicts);
        assert_eq!(s, Summary { pass: 2, fail: 1, not_applicable: 1 });
        assert_eq!(s.total(), 4);
    }

    #[test]
    fn render_verdicts_default_shows_fails_only() {
        let verdicts = vec![
            Verdict::new(Status::Pass, "K01").diagnostic("ok"),
            Verdict::new(Status::Fail, "K02").diagnostic("bad"),
        ];
        let rendered = render_verdicts(&verdicts, false);
        assert!(rendered.contains("K02 FAIL"));
        assert!(!rendered.contains("K01 PASS"));
    }

    #[test]
    fn audit_text_named_matches_audit_file_for_equivalent_input() {
        // The whole point of audit_text_named: an in-memory caller (WASM,
        // a future LSP) must get byte-identical verdicts to what
        // audit_file would produce for the same bytes on disk, given the
        // same two names audit_file itself would have passed in.
        let dir = make_temp_dir("audit_text_matches_audit_file");
        let path = dir.join("bad_frame.s");
        let text = fn_source("bad_frame", 32); // SUB #32 vs ADD #16 -> K01 FAIL
        fs::write(&path, &text).unwrap();

        let from_file = audit_file(&path).unwrap();
        let from_text = audit_text_named(&text, &path.display().to_string(), "bad_frame.s").unwrap();
        assert_eq!(from_file, from_text);
        assert!(from_text.iter().any(|v| v.is_fail()), "fixture should still trip a K01 FAIL");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_text_uses_one_name_for_both_purposes() {
        let text = fn_source("good_fn", 16); // balanced frame -> no K01 FAIL
        let verdicts = audit_text(&text, "pasted.s").unwrap();
        assert!(!verdicts.is_empty());
        for v in &verdicts {
            assert_eq!(v.file.as_deref(), Some("pasted.s"));
        }
    }

    #[test]
    fn audit_text_selected_runs_only_requested_checks() {
        let text = fn_source("bad_frame", 32);
        let selected = audit_text_selected(&text, "bad_frame.s", 1 << 1).unwrap();
        assert!(!selected.is_empty());
        assert!(selected.iter().all(|verdict| verdict.k_class == "K02"));
        let selected = audit_text_selected(&text, "bad_frame.s", 1).unwrap();
        assert!(selected.iter().any(|verdict| verdict.k_class == "K01" && verdict.is_fail()));
    }

    #[test]
    fn audit_text_surfaces_parse_completeness_error_under_the_given_name() {
        // Same dropped-`.type`-declaration shape as
        // parse_s::assert_parse_complete_reports_dropped_type_declaration,
        // routed through audit_text this time to confirm the name passed
        // in shows up in the error exactly like a real path would.
        let text = ".type ghost_fn, %function\n    NOP\n    NOP\n    NOP\n    NOP\nghost_fn:\n    RET\n";
        let err = audit_text(text, "pasted.s").unwrap_err();
        assert!(err.0.starts_with("pasted.s: parse_functions silently dropped"));
    }

    #[test]
    fn simple_glob_matches_star_suffix() {
        assert!(matches_simple_glob("foo.s", "*.s"));
        assert!(!matches_simple_glob("foo.txt", "*.s"));
        assert!(matches_simple_glob("exact", "exact"));
    }

    #[test]
    fn worker_count_is_bounded_and_at_least_one() {
        assert_eq!(worker_count(0), 1);
        assert!(worker_count(1) >= 1);
        assert!(worker_count(1) <= 1); // never more workers than jobs
        assert!(worker_count(10_000) >= 1);
    }

    /// Creates an empty temp directory unique to this test (keyed by name
    /// + pid, since `cargo test` runs tests in this file concurrently by
    /// default and each test needs its own directory).
    fn make_temp_dir(name: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("ufa_baremetal_core_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("failed to create test temp dir");
        dir
    }

    const TEMPLATE_FN: &str = "\
.text
.global {NAME}
.type {NAME}, %function
{NAME}:
    // params: [] ret: VOID
    STP X29, X30, [SP, #-16]!
    SUB SP, SP, #{FRAME}
    MOV X29, SP
    ADD SP, SP, #16
    LDP X29, X30, [SP], #16
    RET
.size {NAME}, . - {NAME}
";

    fn fn_source(name: &str, frame: u32) -> String {
        TEMPLATE_FN.replace("{NAME}", name).replace("{FRAME}", &frame.to_string())
    }

    #[test]
    fn audit_directory_parallel_path_matches_sequential_per_file_audit() {
        let dir = make_temp_dir("parallel_matches_sequential");

        // A handful of files, some clean (frame 16 == balanced SUB/ADD)
        // and some with a K01 frame-imbalance FAIL (SUB #32 vs ADD #16),
        // plus a non-.s file that the glob must exclude.
        fs::write(dir.join("a_good.s"), fn_source("good_a", 16)).unwrap();
        fs::write(dir.join("b_bad.s"), fn_source("bad_b", 32)).unwrap();
        fs::write(dir.join("c_good.s"), fn_source("good_c", 16)).unwrap();
        fs::write(dir.join("d_bad.s"), fn_source("bad_d", 48)).unwrap();
        fs::write(dir.join("notes.txt"), "not assembly, must be excluded by *.s").unwrap();

        let results = audit_directory(&dir, "*.s").expect("directory audit should succeed");

        assert_eq!(results.len(), 4, "the .txt file must be excluded by the glob");
        for name in ["a_good.s", "b_bad.s", "c_good.s", "d_bad.s"] {
            assert!(results.contains_key(name), "missing {name} in results");
        }
        assert!(results["a_good.s"].iter().all(|v| !v.is_fail()));
        assert!(results["c_good.s"].iter().all(|v| !v.is_fail()));
        assert!(results["b_bad.s"].iter().any(|v| v.is_fail()));
        assert!(results["d_bad.s"].iter().any(|v| v.is_fail()));

        // The real point of this test: the parallel path must produce
        // byte-identical verdicts to calling audit_file directly on each
        // path, regardless of which worker thread happened to process it.
        for name in ["a_good.s", "b_bad.s", "c_good.s", "d_bad.s"] {
            let direct = audit_file(&dir.join(name)).unwrap();
            assert_eq!(results[name], direct, "parallel result diverged from direct audit_file for {name}");
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_directory_is_deterministic_across_repeated_runs() {
        let dir = make_temp_dir("deterministic_repeat");
        for i in 0..12 {
            let name = format!("fn_{i:02}.s");
            // Alternate balanced/imbalanced frames so both PASS and FAIL
            // verdicts are exercised across enough files to actually
            // engage multiple worker threads on a multi-core machine.
            let frame = if i % 3 == 0 { 48 } else { 16 };
            fs::write(dir.join(&name), fn_source(&format!("f{i}"), frame)).unwrap();
        }

        let first = audit_directory(&dir, "*.s").unwrap();
        for _ in 0..5 {
            let again = audit_directory(&dir, "*.s").unwrap();
            assert_eq!(first, again, "repeated parallel runs must produce identical output");
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
