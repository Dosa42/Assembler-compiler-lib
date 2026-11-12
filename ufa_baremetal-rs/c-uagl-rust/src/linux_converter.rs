//! Linux-specific conversion shared by the command-line and WebAssembly entry points.

use crate::c_parser::parse_source;
use crate::linux_preprocessor::{get_linux_gvector, preprocess_linux, LinuxPreprocessResult};
use crate::stmt_converter::{clear_gaps, take_gaps, Gap};
use crate::type_map::{kappa, tessa_class, GVector};
use crate::uagl_emitter::emit_uagl;
use serde_json::{json, Map, Value};
use std::{borrow::Cow, io, path::Path};

pub const VERSION: &str = "1.0.0";

pub type ConversionError = io::Error;

pub struct ConvertedSource {
    pub uagl: String,
    pub gaps: Vec<Gap>,
    pub has_module: bool,
    pub warnings: Vec<String>,
    pub lines_in: usize,
    pub lines_out: usize,
}

pub fn normalize_newlines(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

fn filename(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
}

fn emit_linux_module_header(result: &LinuxPreprocessResult, gv: GVector, path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_uppercase()
        .replace(['-', '.'], "_");
    let k = kappa(&gv);
    let tc = tessa_class(k);
    let (g1, g2, g3, g4) = gv;

    let mut lines = vec![
        format!("/* UAGL v3 — Linux kernel: {}", filename(path)),
        format!("   G-Vector: g1={g1:?} g2={g2:?} g3={g3:?} g4={g4:?}"),
        format!("   κ={k:.4}  TESSA={tc}"),
        format!("   Converter: linux_converter v{VERSION}"),
    ];
    if let Some(name) = &result.module_init {
        lines.push(format!("   MODULE_INIT: {name}"));
    }
    if let Some(name) = &result.module_exit {
        lines.push(format!("   MODULE_EXIT: {name}"));
    }
    lines.extend(
        [
            "*/",
            "",
            "#PRAGMA BIOS64",
            "#PRAGMA FREESTANDING",
            "#PRAGMA OPTIMIZE 3",
            "",
        ]
        .map(str::to_owned),
    );

    if result.module_init.is_some() || result.module_exit.is_some() {
        lines.push("/* ── MODULE LIFECYCLE ── */".to_owned());
        lines.push(format!("╔═ MODULE_{stem} ═╗"));
        if let Some(name) = &result.module_init {
            lines.push(format!("║  · __init_func ─ PTR ─ {name} ◇"));
        }
        if let Some(name) = &result.module_exit {
            lines.push(format!("║  · __exit_func ─ PTR ─ {name} ◇"));
        }
        lines.push("╚═══ 1 ═══╝".to_owned());
        lines.push(String::new());
    }
    lines.join("\n")
}

pub fn convert_source(source: &str, source_path: &str) -> Result<ConvertedSource, ConversionError> {
    let source = normalize_newlines(source);
    let lines_in = source.matches('\n').count();
    let preprocessed = preprocess_linux(&source);
    let path = Path::new(source_path);

    // The Python driver parses through a temporary C filename, then restores
    // the source basename before the emitter sees the translation unit.
    let mut unit = parse_source(&preprocessed.src, "tmp.c")?;
    unit.filename = filename(path).to_owned();

    clear_gaps();
    let gv = get_linux_gvector(source_path);
    let base = emit_uagl(&unit, source_path)?;
    let gaps = take_gaps();
    let cut = base
        .split_inclusive('\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        })
        .find(|(offset, line)| *offset > 0 && line.starts_with("╔═"))
        .map_or(0, |(offset, _)| offset);
    let mut uagl = emit_linux_module_header(&preprocessed, gv, path);
    uagl.push_str(&base[cut..]);
    let lines_out = uagl.matches('\n').count();

    Ok(ConvertedSource {
        uagl,
        gaps,
        has_module: preprocessed.module_init.is_some() || preprocessed.module_exit.is_some(),
        warnings: preprocessed.warnings,
        lines_in,
        lines_out,
    })
}

pub fn needs_ai(gaps: &[Gap]) -> usize {
    gaps.iter().filter(|gap| gap.severity == "needs-ai").count()
}

pub fn gap_manifest(source_path: &str, output_path: &str, gaps: &[Gap]) -> String {
    let mut counts = Map::new();
    let records: Vec<Value> = gaps
        .iter()
        .map(|gap| {
            let count = counts.entry(gap.kind.clone()).or_insert(json!(0));
            *count = json!(count.as_u64().unwrap_or_default() + 1);
            json!({
                "id": gap.id,
                "function": gap.function,
                "kind": gap.kind,
                "severity": gap.severity,
                "c": gap.c,
                "note": gap.note,
            })
        })
        .collect();
    let manifest = json!({
        "source": source_path,
        "output": output_path,
        "gap_count": gaps.len(),
        "needs_ai": needs_ai(gaps),
        "by_kind": counts,
        "gaps": records,
    });
    serde_json::to_string_pretty(&manifest).expect("manifest contains only JSON values")
}
