//! `uagl_aarch64_assembler` — a faithful Rust port of
//! `aarch64_assembler_braille.py` (UAGL v3 → AArch64 GNU assembler).
//!
//! Module layout mirrors the original file's logical sections:
//! - [`token`] / [`lexer`] — tokenizer (`TK`, `Token`, `lex_uagl`)
//! - [`tables`] — process-lifetime global state + static lookup tables
//! - [`types`] — type sizing, alignment, struct layout
//! - [`func_state`] — per-function codegen state (`FuncState`, `reg_for`)
//! - [`helpers`] — small parsing/string utilities
//! - [`expr`] — expression codegen (`emit_load_value` and friends)
//! - [`call`] — function-call / comparison / scope codegen
//! - [`codegen`] — `AArch64Generator`, the main token-stream compiler
//! - [`uaglbr`] — the UAGL Braille Latin container format
//!
//! Public API: [`assemble_uagl_str`], [`assemble_uagl_bytes`], [`validate_uagl_bytes`],
//! [`assemble_file`], [`assemble_batch`].

pub mod call;
pub mod codegen;
mod diagnostics;
pub mod expr;
pub mod func_state;
pub mod helpers;
pub mod lexer;
pub mod tables;
pub mod token;
pub mod types;
pub mod uaglbr;

use codegen::AArch64Generator;
pub use diagnostics::SourceDiagnostic;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

/// Compile UAGL source text to AArch64 assembly. Mirrors the `str` arm of
/// `assemble_uagl`.
pub fn assemble_uagl_str(uagl_src: &str, source_name: &str) -> String {
    let tokens = lexer::lex_uagl(uagl_src);
    let mut gen = AArch64Generator::new(source_name.to_string());
    gen.gen(&tokens)
}

/// Compile raw UAGL source bytes, or a canonical `.uaglbr` container, to
/// AArch64 assembly. Mirrors the `bytes`/`bytearray`/`memoryview` arm of
/// `assemble_uagl`. (Rust's static typing splits Python's single
/// `str | bytes` overload into two entry points; `assemble_uagl_str`
/// above is the other half.)
pub fn assemble_uagl_bytes(uagl_src: &[u8], source_name: &str) -> Result<String, String> {
    let (text, source_name) = decode_uagl_input(uagl_src, source_name, true)?;
    Ok(assemble_uagl_str(&text, &source_name))
}

/// Report unrecognized UAGL lines and expressions the assembler could not
/// translate. This runs the source through the existing assembler's lexer and
/// code generator without returning assembly. Like assembly, validation may
/// populate the process-wide struct and global tables.
pub fn validate_uagl_bytes(uagl_src: &[u8], source_name: &str) -> Result<Vec<SourceDiagnostic>, String> {
    let (text, source_name) = decode_uagl_input(uagl_src, source_name, false)?;
    let tokens = lexer::lex_for_validation(&text);
    let mut generator = AArch64Generator::new(source_name);
    Ok(diagnostics::collect(|| generator.validate(&tokens)))
}

fn decode_uagl_input(
    uagl_src: &[u8],
    source_name: &str,
    allow_lossy_utf8: bool,
) -> Result<(String, String), String> {
    let has_magic = uaglbr::has_uaglbr_magic(uagl_src);
    let mut source_name = source_name.to_string();
    if source_name.ends_with(".uaglbr") && !has_magic {
        return Err("invalid UAGL Braille container magic".to_string());
    }
    let text = if has_magic {
        let raw = uaglbr::decode_uaglbr_bytes(uagl_src)?;
        if source_name.ends_with(".uaglbr") {
            source_name.truncate(source_name.len() - ".uaglbr".len());
            if !source_name.ends_with(".uagl") {
                source_name.push_str(".uagl");
            }
        }
        String::from_utf8(raw).map_err(|e| e.to_string())?
    } else if allow_lossy_utf8 {
        String::from_utf8_lossy(uagl_src).to_string()
    } else {
        std::str::from_utf8(uagl_src)
            .map_err(|e| format!("Invalid UTF-8 in UAGL source: {e}"))?
            .to_string()
    };
    Ok((text, source_name))
}

#[derive(Debug, Clone)]
pub struct AssembleFileResult {
    pub src: String,
    pub out: String,
    pub source_name: String,
    pub uaglbr_decoded: bool,
    pub lines_in: usize,
    pub lines_out: usize,
}

/// Load, compile, and write one `.uagl`/`.uaglbr` file to a `.s` output
/// path, creating parent directories as needed. Mirrors `assemble_file`.
pub fn assemble_file(src_path: &str, out_path: &str) -> Result<AssembleFileResult, String> {
    let (src, source_name, was_uaglbr) = uaglbr::load_uagl_source(src_path)?;
    let asm = assemble_uagl_str(&src, &source_name);
    if let Some(parent) = Path::new(out_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(out_path, &asm).map_err(|e| e.to_string())?;
    Ok(AssembleFileResult {
        src: src_path.to_string(),
        out: out_path.to_string(),
        source_name,
        uaglbr_decoded: was_uaglbr,
        lines_in: src.matches('\n').count(),
        lines_out: asm.matches('\n').count(),
    })
}

#[derive(Debug, Clone)]
pub struct BatchResult {
    pub ok: usize,
    pub errors: usize,
    pub total_asm_lines: usize,
}

fn collect_uagl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_uagl_files(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext == "uagl" || ext == "uaglbr" {
                out.push(path);
            }
        }
    }
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::new();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Recursively compile every `.uagl`/`.uaglbr` file under `uagl_dir` to a
/// mirrored `.s` file under `out_dir`. Mirrors `assemble_batch`: a
/// sequential, error-ignoring struct-discovery pass over every file runs
/// first (so cross-file struct dependencies resolve correctly regardless of
/// directory order), then the real compile runs in parallel over a small
/// worker-thread pool. `workers` sizes that second pass's pool (unlike the
/// Python original, which now ignores `workers` and compiles Pass 2
/// sequentially too -- Rust's `Mutex`-guarded tables make concurrent Pass-2
/// compilation safe here, so this port keeps genuine parallelism for it).
pub fn assemble_batch(uagl_dir: &str, out_dir: &str, workers: usize) -> BatchResult {
    let mut files = Vec::new();
    collect_uagl_files(Path::new(uagl_dir), &mut files);
    files.sort();
    files.dedup();
    let total = files.len();

    println!("Assembling {} UAGL files → AArch64...", total);

    // --- Pass 1 (discovery): populate `tables::STRUCT_TABLE` from every
    // file in the batch, sequentially, before any real compile. Output is
    // discarded and both `Err`s and panics are ignored here -- this mirrors
    // the Python original's `except Exception: pass` discovery sweep (see
    // `assemble_batch`'s docstring in aarch64_assembler_braille.py). Some
    // paths in this compiler deliberately `panic!` to mirror the original's
    // uncaught exceptions (see expr::parse_base0_int), so a plain `Result`
    // isn't enough to ignore a bad file here -- `catch_unwind` is required.
    //
    // Without this pass, a struct *used* in one file (as a field, a local,
    // or a global element type) but *defined* in another file that happens
    // to sort/get-popped later would have its layout memoized as an
    // "unknown struct" fallback the first time anything asks for it
    // (`tables::STRUCT_LAYOUTS`), and that wrong layout is never
    // recomputed -- silently undersizing every buffer of that struct type
    // for the rest of the batch. This is the same hazard the C port's
    // `assemble_batch_c` fixes the same way (see UAGL_C_PORT_AUDIT_FINAL.md:
    // an observed 30,720-byte `Poly[30]` global coming out as a 0-byte
    // allocation). Confirmed independently: with this pass removed, a
    // `Big[10]` global defined via a struct declared in a separate,
    // later-processed file compiled to `.zero 80` instead of the correct
    // `.zero 320`.
    for f in &files {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Ok((src, source_name, _)) = uaglbr::load_uagl_source(&f.to_string_lossy()) {
                let _ = assemble_uagl_str(&src, &source_name);
            }
        }));
    }

    // Discard any layout computed during discovery -- some of it was
    // necessarily wrong (computed before every struct was known). Struct
    // *definitions* in STRUCT_TABLE (and any resolved GLOBAL_TABLE entries)
    // are unaffected and stay populated, exactly like the Python original.
    tables::STRUCT_LAYOUTS.lock().unwrap().clear();

    // --- Pass 2 (real compile): every struct in the batch is now known, so
    // layout computation is order-independent no matter which worker thread
    // reaches which file first. This part keeps the existing worker-pool
    // parallelism -- only Pass 1 needs to be sequential.
    let uagl_dir_owned = uagl_dir.to_string();
    let out_dir_owned = out_dir.to_string();
    let work_queue: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(files));
    let results: Arc<Mutex<Vec<Result<AssembleFileResult, String>>>> = Arc::new(Mutex::new(Vec::new()));

    thread::scope(|scope| {
        for _ in 0..workers.max(1) {
            let work_queue = Arc::clone(&work_queue);
            let results = Arc::clone(&results);
            let uagl_dir_owned = uagl_dir_owned.clone();
            let out_dir_owned = out_dir_owned.clone();
            scope.spawn(move || loop {
                let f = { work_queue.lock().unwrap().pop() };
                let f = match f {
                    Some(f) => f,
                    None => break,
                };
                let rel = f.strip_prefix(&uagl_dir_owned).unwrap_or(&f).to_path_buf();
                let mut out = PathBuf::from(&out_dir_owned).join(&rel);
                out.set_extension("s");
                let r = assemble_file(&f.to_string_lossy(), &out.to_string_lossy());
                results.lock().unwrap().push(r);
            });
        }
    });

    let results = results.lock().unwrap();
    let errors = results.iter().filter(|r| r.is_err()).count();
    let ok = results.len() - errors;
    let total_asm_lines: usize = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .map(|r| r.lines_out)
        .sum();

    println!("  OK: {}/{} | Errors: {}", ok, total, errors);
    println!("  Total ASM lines: {}", thousands(total_asm_lines));

    BatchResult { ok, errors, total_asm_lines }
}
