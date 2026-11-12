//! Port of `cli.py` / `__main__.py`.
//!
//! `ufa-baremetal <path> [flags]` — runs every K-check in
//! `K_REGISTRY` against a single `.s` file or every `.s` file directly
//! under a directory, and prints one diagnostic line per FAIL (plus an
//! indented `fix:` continuation line where a check has a suggested fix).
//!
//! Exit codes: 0 = no FAILs, 1 = at least one FAIL, 2 = usage / IO /
//! parse-completeness error.
//!
//! `ufa-baremetal bug-coverage` — prints the [`ufa_baremetal_core::regression_map`]
//! coverage report (a Rust equivalent of `python -m ufa_baremetal.regression_map`).
//!
//! `ufa-baremetal assemble <file.uagl> [--out <path>] [--check]` — compiles
//! a UAGL source file to AArch64 assembly via the `uagl_aarch64_assembler`
//! crate (a workspace member, not a subprocess or FFI call) and, with
//! `--check`, immediately runs the same K01-K06 pipeline above against the
//! freshly generated text via
//! [`ufa_baremetal_core::codegen_audit::audit_text`] — no temp file
//! required for the check step. This is the "compiler and checker in one
//! binary" path: both crates are ordinary `path = ".."` workspace
//! dependencies of this one CLI, compiled together by a single
//! `cargo build`.
//!
//! This binary depends on nothing beyond `ufa_baremetal_core` and
//! `uagl_aarch64_assembler` — no `clap`, no `serde` of its own. The flag
//! set is small enough that hand-rolled parsing is both less code and less
//! risk than pulling in an argument-parsing crate.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ufa_baremetal_core::codegen_audit::{self, AuditError};
use ufa_baremetal_core::json::Json;
use ufa_baremetal_core::k_invariants;
use ufa_baremetal_core::regression_map;
use ufa_baremetal_core::sha256;
use ufa_baremetal_core::verdict::{Status, Verdict};
use ufa_baremetal_core::datetime::utc_now_iso8601;

const TOOL_VERSION: &str = "ufa_baremetal-1.0 (rust port)";

const EXIT_OK: u8 = 0;
const EXIT_FAIL: u8 = 1;
const EXIT_USAGE: u8 = 2;

struct Args {
    path: PathBuf,
    show_pass: bool,
    summary: bool,
    k_filter: String,
    json: bool,
}

fn print_usage() {
    println!(
        "ufa_baremetal: K01-K06 invariant-check harness. Runs the checks in
K_REGISTRY over emitted AArch64 assembly.

USAGE:
  ufa-baremetal <path> [FLAGS]
  ufa-baremetal bug-coverage
  ufa-baremetal assemble <file.uagl|.uaglbr> [--out <path>] [--check] [--show-pass]

ARGS:
  <path>            Path to a single .s file or a directory of .s files.

FLAGS:
  --show-pass       Also print PASS / NOT_APPLICABLE verdicts (default: only FAIL).
  --summary         Print a per-file PASS/FAIL/N-A count summary at the end.
  --k K01,K02,...   Restrict to a subset of K-checks (default: all).
  --json            Emit verdicts as a single JSON document on stdout.
                     Mutually exclusive with --show-pass and --summary.
  -h, --help        Print this help and exit.

ASSEMBLE SUBCOMMAND:
  Compiles a .uagl/.uaglbr source file to AArch64 assembly in-process
  (via the uagl_aarch64_assembler workspace crate -- no subprocess).
  --out <path>      Write the generated assembly here (default: stdout).
  --check           Also run the K01-K06 checks against the generated
                     assembly before writing/printing it, and print any
                     FAIL verdicts (plus --show-pass for PASS/N-A)."
    );
}

/// Returns `None` on a usage error (already printed to stderr).
fn parse_args(argv: &[String]) -> Option<Args> {
    let mut path: Option<PathBuf> = None;
    let mut show_pass = false;
    let mut summary = false;
    let mut k_filter = String::new();
    let mut json = false;

    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        match a {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(EXIT_OK as i32);
            }
            "--show-pass" => show_pass = true,
            "--summary" => summary = true,
            "--json" => json = true,
            "--k" => {
                i += 1;
                if i >= argv.len() {
                    eprintln!("ufa_baremetal: error: --k requires a value");
                    return None;
                }
                k_filter = argv[i].clone();
            }
            _ if a.starts_with("--k=") => {
                k_filter = a["--k=".len()..].to_string();
            }
            _ if a.starts_with('-') && a != "-" => {
                eprintln!("ufa_baremetal: error: unrecognized flag: {a}");
                return None;
            }
            _ => {
                if path.is_some() {
                    eprintln!("ufa_baremetal: error: unexpected extra argument: {a}");
                    return None;
                }
                path = Some(PathBuf::from(a));
            }
        }
        i += 1;
    }

    let Some(path) = path else {
        eprintln!("ufa_baremetal: error: missing required argument <path>");
        print_usage();
        return None;
    };

    Some(Args { path, show_pass, summary, k_filter, json })
}

/// Mirrors `_resolve_k_subset()` in `cli.py`. `Ok(None)` means "all
/// checks" (no `--k` given); `Err(())` means an error was already
/// printed.
fn resolve_k_subset(k_arg: &str) -> Result<Option<Vec<&'static str>>, ()> {
    if k_arg.is_empty() {
        return Ok(None);
    }
    let all: Vec<&'static str> = k_invariants::names();
    let mut requested: Vec<String> =
        k_arg.split(',').map(|p| p.trim().to_uppercase()).filter(|p| !p.is_empty()).collect();
    requested.sort();
    requested.dedup();

    let mut unknown: Vec<String> = requested.iter().filter(|r| !all.contains(&r.as_str())).cloned().collect();
    unknown.sort();
    if !unknown.is_empty() {
        eprintln!(
            "ufa_baremetal: error: unknown K-class(es): {}. Known: {}.",
            unknown.join(", "),
            all.join(", ")
        );
        return Err(());
    }

    let selected: Vec<&'static str> = all.into_iter().filter(|k| requested.contains(&k.to_string())).collect();
    Ok(Some(selected))
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    if argv.first().map(String::as_str) == Some("bug-coverage") {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        if regression_map::print_coverage_report(&mut lock).is_err() {
            return ExitCode::from(EXIT_USAGE);
        }
        return ExitCode::from(EXIT_OK);
    }

    if argv.first().map(String::as_str) == Some("assemble") {
        return handle_assemble(&argv[1..]);
    }

    let Some(args) = parse_args(&argv) else {
        return ExitCode::from(EXIT_USAGE);
    };

    if args.json && (args.show_pass || args.summary) {
        eprintln!("ufa_baremetal: error: --json is mutually exclusive with --show-pass and --summary");
        return ExitCode::from(EXIT_USAGE);
    }

    if !args.path.exists() {
        eprintln!("ufa_baremetal: error: path not found: {}", args.path.display());
        return ExitCode::from(EXIT_USAGE);
    }

    let selected = match resolve_k_subset(&args.k_filter) {
        Ok(s) => s,
        Err(()) => return ExitCode::from(EXIT_USAGE),
    };

    let results: BTreeMap<String, Vec<Verdict>> = if args.path.is_file() {
        match codegen_audit::audit_file(&args.path) {
            Ok(verdicts) => {
                let name = args.path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
                BTreeMap::from([(name, verdicts)])
            }
            Err(e) => return handle_audit_error(e),
        }
    } else if args.path.is_dir() {
        match codegen_audit::audit_directory(&args.path, "*.s") {
            Ok(r) => r,
            Err(e) => return handle_audit_error(e),
        }
    } else {
        eprintln!("ufa_baremetal: error: not a file or directory: {}", args.path.display());
        return ExitCode::from(EXIT_USAGE);
    };

    let results: BTreeMap<String, Vec<Verdict>> = match &selected {
        Some(sel) => results
            .into_iter()
            .map(|(f, vs)| (f, vs.into_iter().filter(|v| sel.contains(&v.k_class)).collect()))
            .collect(),
        None => results,
    };

    let mut any_fail = false;
    for verdicts in results.values() {
        if verdicts.iter().any(Verdict::is_fail) {
            any_fail = true;
        }
        if !args.json {
            for v in verdicts {
                if v.is_fail() {
                    println!("{}", v.render());
                } else if args.show_pass && (v.is_pass() || v.status == Status::NotApplicable) {
                    println!("{}", v.render());
                }
            }
        }
    }

    if args.summary {
        println!();
        println!("─── summary ─────────────────────────────────────");
        let mut total_files = 0usize;
        let mut clean_files = 0usize;
        for (fname, verdicts) in &results {
            total_files += 1;
            let s = codegen_audit::summarize(verdicts);
            if s.fail == 0 {
                clean_files += 1;
            }
            println!("  {fname:<32} PASS={:>3} FAIL={:>2} N/A={:>2}", s.pass, s.fail, s.not_applicable);
        }
        println!("  {:<32}   {clean_files}/{total_files} files clean", "─".repeat(32));
    }

    if args.json {
        emit_json(&args.path, &results);
    }

    if any_fail {
        ExitCode::from(EXIT_FAIL)
    } else {
        ExitCode::from(EXIT_OK)
    }
}

fn handle_audit_error(e: AuditError) -> ExitCode {
    match e {
        AuditError::ParseCompleteness(pe) => {
            eprintln!("ufa_baremetal: parse-completeness preflight FAILED.");
            eprintln!("  {pe}");
        }
        other => {
            eprintln!("ufa_baremetal: error: {other}");
        }
    }
    ExitCode::from(EXIT_USAGE)
}

/// Mirrors `_emit_json()` in `cli.py`. Determinism: file/verdict order
/// matches [`codegen_audit::audit_directory`]'s sorted iteration (a
/// `BTreeMap`, walked in key order below); the only non-deterministic
/// field is `generated_at`.
fn emit_json(target: &Path, results: &BTreeMap<String, Vec<Verdict>>) {
    let files: Vec<PathBuf> = if target.is_file() {
        vec![target.to_path_buf()]
    } else {
        let mut v: Vec<PathBuf> = std::fs::read_dir(target)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("s"))
            .collect();
        v.sort();
        v
    };

    let mut inputs = Vec::new();
    for f in &files {
        let Ok(data) = std::fs::read(f) else { continue };
        let name = f.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let path = std::fs::canonicalize(f).unwrap_or_else(|_| f.clone());
        inputs.push(Json::Object(vec![
            ("name".into(), Json::str(name)),
            ("path".into(), Json::str(path.display().to_string())),
            ("sha256".into(), Json::str(sha256::sha256_hex(&data))),
            ("bytes".into(), Json::UInt(data.len() as u64)),
        ]));
    }

    let mut verdicts_out = Vec::new();
    let mut summary_pass = 0u64;
    let mut summary_fail = 0u64;
    let mut summary_na = 0u64;
    for verdicts in results.values() {
        for v in verdicts {
            verdicts_out.push(Json::Object(vec![
                ("status".into(), Json::str(v.status.as_str())),
                ("k_class".into(), Json::str(v.k_class)),
                ("file".into(), Json::opt_str(v.file.clone())),
                ("function".into(), Json::opt_str(v.function.clone())),
                ("line_no".into(), Json::opt_u32(v.line_no)),
                ("diagnostic".into(), Json::str(v.diagnostic.clone())),
                ("suggested_fix".into(), Json::str(v.suggested_fix.clone())),
            ]));
            match v.status {
                Status::Pass => summary_pass += 1,
                Status::Fail => summary_fail += 1,
                Status::NotApplicable => summary_na += 1,
            }
        }
    }

    let doc = Json::Object(vec![
        ("tool_version".into(), Json::str(TOOL_VERSION)),
        ("generated_at".into(), Json::str(utc_now_iso8601())),
        ("inputs".into(), Json::Array(inputs)),
        ("verdicts".into(), Json::Array(verdicts_out)),
        (
            "summary".into(),
            Json::Object(vec![
                ("PASS".into(), Json::UInt(summary_pass)),
                ("FAIL".into(), Json::UInt(summary_fail)),
                ("NOT_APPLICABLE".into(), Json::UInt(summary_na)),
            ]),
        ),
    ]);

    println!("{}", doc.to_pretty_string());
}

struct AssembleArgs {
    src: PathBuf,
    out: Option<PathBuf>,
    check: bool,
    show_pass: bool,
}

/// Mirrors the top-level flag parser's shape/style, scoped to the
/// `assemble` subcommand's own small flag set.
fn parse_assemble_args(argv: &[String]) -> Option<AssembleArgs> {
    let mut src: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut check = false;
    let mut show_pass = false;

    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        match a {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(EXIT_OK as i32);
            }
            "--out" => {
                i += 1;
                if i >= argv.len() {
                    eprintln!("ufa_baremetal assemble: error: --out requires a value");
                    return None;
                }
                out = Some(PathBuf::from(&argv[i]));
            }
            "--check" => check = true,
            "--show-pass" => show_pass = true,
            _ if a.starts_with('-') && a != "-" => {
                eprintln!("ufa_baremetal assemble: error: unrecognized flag: {a}");
                return None;
            }
            _ => {
                if src.is_some() {
                    eprintln!("ufa_baremetal assemble: error: unexpected extra argument: {a}");
                    return None;
                }
                src = Some(PathBuf::from(a));
            }
        }
        i += 1;
    }

    let Some(src) = src else {
        eprintln!("ufa_baremetal assemble: error: missing required argument <file.uagl|.uaglbr>");
        return None;
    };

    Some(AssembleArgs { src, out, check, show_pass })
}

/// `ufa-baremetal assemble` — compiles UAGL source to AArch64 assembly via
/// the embedded `uagl_aarch64_assembler` crate (an ordinary in-process
/// function call: no subprocess, no FFI, no temp-file bridge to another
/// language) and, with `--check`, feeds the resulting text straight into
/// [`codegen_audit::audit_text`] before it ever touches disk.
fn handle_assemble(argv: &[String]) -> ExitCode {
    let Some(args) = parse_assemble_args(argv) else {
        return ExitCode::from(EXIT_USAGE);
    };

    if !args.src.exists() {
        eprintln!("ufa_baremetal assemble: error: path not found: {}", args.src.display());
        return ExitCode::from(EXIT_USAGE);
    }

    let raw = match std::fs::read(&args.src) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ufa_baremetal assemble: error: reading {}: {e}", args.src.display());
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let source_name =
        args.src.file_name().and_then(|s| s.to_str()).unwrap_or("input.uagl").to_string();

    // The assembler's core codegen deliberately panics on some
    // malformed-input paths, mirroring the Python original's uncaught
    // exceptions -- catch that here for a clean CLI error instead of a
    // raw backtrace (same technique `assemble_batch`'s discovery pass
    // uses internally).
    let asm = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        uagl_aarch64_assembler::assemble_uagl_bytes(&raw, &source_name)
    }));
    let asm = match asm {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => {
            eprintln!("ufa_baremetal assemble: error: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
        Err(_) => {
            eprintln!(
                "ufa_baremetal assemble: error: {} failed to compile (malformed UAGL source)",
                args.src.display()
            );
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let mut any_fail = false;
    if args.check {
        match codegen_audit::audit_text(&asm, &source_name) {
            Ok(verdicts) => {
                for v in &verdicts {
                    if v.is_fail() {
                        any_fail = true;
                        println!("{}", v.render());
                    } else if args.show_pass && (v.is_pass() || v.status == Status::NotApplicable) {
                        println!("{}", v.render());
                    }
                }
                let s = codegen_audit::summarize(&verdicts);
                println!(
                    "─── {source_name}: PASS={} FAIL={} N/A={} ───",
                    s.pass, s.fail, s.not_applicable
                );
            }
            Err(pe) => {
                eprintln!("ufa_baremetal assemble: parse-completeness preflight FAILED on generated assembly.");
                eprintln!("  {pe}");
                eprintln!("  (this would point at a bug in uagl_aarch64_assembler's codegen, not in the K-checks)");
                return ExitCode::from(EXIT_USAGE);
            }
        }
    }

    match &args.out {
        Some(out_path) => {
            if let Some(parent) = out_path.parent() {
                if !parent.as_os_str().is_empty() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        eprintln!("ufa_baremetal assemble: error: creating {}: {e}", parent.display());
                        return ExitCode::from(EXIT_USAGE);
                    }
                }
            }
            if let Err(e) = std::fs::write(out_path, &asm) {
                eprintln!("ufa_baremetal assemble: error: writing {}: {e}", out_path.display());
                return ExitCode::from(EXIT_USAGE);
            }
            eprintln!("Output: {} ({} lines)", out_path.display(), asm.matches('\n').count());
        }
        None => {
            print!("{asm}");
        }
    }

    if any_fail {
        ExitCode::from(EXIT_FAIL)
    } else {
        ExitCode::from(EXIT_OK)
    }
}
