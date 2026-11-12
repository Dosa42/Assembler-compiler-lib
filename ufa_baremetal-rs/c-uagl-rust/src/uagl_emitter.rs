//! Emit UAGL v3 source from a parsed C translation unit.

use crate::c_parser::{CDefine, CFunction, CStruct, CUnit, CVar};
use crate::stmt_converter::{convert_body, set_current_func, split_top};
use crate::type_map::{get_gvector, kappa, map_array_type, map_type, tessa_class, GVector};
use regex::Regex;
use std::collections::HashMap;
use std::io;
use std::sync::LazyLock;

macro_rules! regex {
    ($pattern:literal) => {{
        static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new($pattern).unwrap());
        &*RE
    }};
}

pub const OP_MAP: &[(&str, &str)] = &[("!=", "╱"), ("&&", "∧"), (r"\|\|", "∨")];

fn is_python_space(ch: char) -> bool {
    ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch)
}

fn strip(text: &str) -> &str {
    text.trim_matches(is_python_space)
}

pub fn conv_expr(expr: &str) -> String {
    let mut expr = strip(expr);
    if let Some(inner) = expr.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        let mut depth = 0;
        let balanced = inner.chars().all(|ch| {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            depth >= 0
        });
        if balanced {
            expr = inner;
        }
    }

    let arrows = regex!(r"([\pL\pN_]+)[\s\x1c-\x1f]*->[\s\x1c-\x1f]*([\pL\pN_]+)")
        .replace_all(expr, "${1} ─ △ ${2}");
    let fields = regex!(r"([\pL\pN_]+)\.([\pL\pN_]+)").replace_all(&arrows, "${1} ─ △ ${2}");
    let operators = fields
        .replace("!=", "╱")
        .replace("&&", "∧")
        .replace("||", "∨");
    regex!(r"![\s\x1c-\x1f]*([\pL\pN_])")
        .replace_all(&operators, "!${1}")
        .replace("true", "1")
        .replace("false", "0")
        .replace("NULLPTR", "NULL")
}

pub fn conv_statements(
    body: &str,
    indent: usize,
    param_types: Option<&HashMap<String, String>>,
    ret_type: &str,
) -> Vec<String> {
    convert_body(body, indent, param_types, ret_type)
}

/// The original line-by-line converter, retained alongside the active statement converter.
pub fn conv_statements_legacy(body: &str, indent: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let body = strip(body);
    if body.is_empty() {
        return lines;
    }
    let pad = "║  ".repeat(indent);
    let next_indent = indent + 1;
    let raw_lines: Vec<_> = body.split('\n').collect();
    let mut i = 0;
    while i < raw_lines.len() {
        let line = strip(raw_lines[i]);
        i += 1;

        if line.is_empty() || line.starts_with('#') || matches!(line, "{" | "}" | "};") {
            continue;
        }
        if line.starts_with("/*") || line.starts_with("//") {
            let comment = strip(
                line.trim_start_matches(['/', '*'])
                    .trim_end_matches(['*', '/']),
            );
            lines.push(format!("{pad}/* {comment} */"));
            continue;
        }
        if let Some(expr) = line.strip_prefix("return") {
            let expr = strip(expr.trim_end_matches(';'));
            if expr.is_empty() {
                lines.push(format!("{pad}→ ◇"));
            } else {
                lines.push(format!("{pad}→ {} ◇", conv_expr(expr)));
            }
            continue;
        }
        if line.starts_with("if") || line.starts_with("} else if") || line.starts_with("else if") {
            if let Some(cond) =
                regex!(r"^(?:}\s*)?(?:else\s+)?if\s*\(\s*(.*?)\s*\)\s*(?:\{|$)").captures(line)
            {
                let cond = conv_expr(&cond[1]);
                let prefix = if line.contains("else") {
                    "▽ Φ"
                } else {
                    "Φ"
                };
                lines.push(format!("{pad}{prefix} {cond} ═╗"));
                let (body_lines, next) = collect_block(&raw_lines, i);
                i = next;
                lines.extend(conv_statements(
                    &body_lines.join("\n"),
                    next_indent,
                    None,
                    "INT64",
                ));
                lines.push(format!("{pad}╚═══ {next_indent} ═══╝"));
            }
            continue;
        }
        if line.starts_with("} else") || line.starts_with("else") {
            lines.push(format!("{pad}▽ ═╗"));
            let (body_lines, next) = collect_block(&raw_lines, i);
            i = next;
            lines.extend(conv_statements(
                &body_lines.join("\n"),
                next_indent,
                None,
                "INT64",
            ));
            lines.push(format!("{pad}╚═══ {next_indent} ═══╝"));
            continue;
        }
        if line.starts_with("while") {
            if let Some(cond) = regex!(r"^while\s*\(\s*(.*?)\s*\)\s*(?:\{|$)").captures(line) {
                let cond = conv_expr(&cond[1]);
                lines.push(format!("{pad}ⁿ {cond} ═╗"));
                let (body_lines, next) = collect_block(&raw_lines, i);
                i = next;
                lines.extend(conv_statements(
                    &body_lines.join("\n"),
                    next_indent,
                    None,
                    "INT64",
                ));
                lines.push(format!("{pad}╚═══ {next_indent} ═══╝"));
            }
            continue;
        }
        if line.starts_with("for") {
            if let Some(parts) =
                regex!(r"^for\s*\(\s*(.*?);\s*(.*?);\s*(.*?)\s*\)\s*(?:\{|$)").captures(line)
            {
                let init = strip(&parts[1]);
                let cond = conv_expr(strip(&parts[2]));
                let incr = strip(&parts[3]);
                if !init.is_empty() {
                    lines.extend(conv_local_decl(init, &pad));
                }
                lines.push(format!("{pad}ⁿ {cond} ═╗"));
                let (body_lines, next) = collect_block(&raw_lines, i);
                i = next;
                lines.extend(conv_statements(
                    &body_lines.join("\n"),
                    next_indent,
                    None,
                    "INT64",
                ));
                if !incr.is_empty() {
                    lines.extend(conv_assignment(incr, &"║  ".repeat(next_indent)));
                }
                lines.push(format!("{pad}╚═══ {next_indent} ═══╝"));
            }
            continue;
        }
        if line.starts_with("do") {
            lines.push(format!("{pad}ⁿ 1 ═╗"));
            let (body_lines, next) = collect_block(&raw_lines, i);
            i = next;
            lines.extend(conv_statements(
                &body_lines.join("\n"),
                next_indent,
                None,
                "INT64",
            ));
            if let Some(cond) = regex!(r"^\s*while\s*\(\s*(.*?)\s*\)\s*;")
                .captures(raw_lines.get(i).copied().unwrap_or(""))
            {
                let cond = conv_expr(&cond[1]);
                let next_pad = "║  ".repeat(next_indent);
                let nested_pad = "║  ".repeat(next_indent + 1);
                lines.push(format!("{next_pad}Φ !({cond}) ═╗"));
                lines.push(format!("{nested_pad}→ ◇"));
                lines.push(format!("{next_pad}╚═══ {} ═══╝", next_indent + 1));
                i += 1;
            }
            lines.push(format!("{pad}╚═══ {next_indent} ═══╝"));
            continue;
        }
        if line.starts_with("switch") {
            if let Some(parts) = regex!(r"^switch\s*\(\s*(.*?)\s*\)\s*\{?").captures(line) {
                let sw_expr = conv_expr(&parts[1]);
                lines.push(format!("{pad}/* switch {sw_expr} */"));
                let (body_lines, next) = collect_block(&raw_lines, i);
                i = next;
                lines.extend(conv_switch(&body_lines.join("\n"), &sw_expr, indent, &pad));
            }
            continue;
        }

        let declaration = regex!(
            r"^((?:const\s+|volatile\s+|static\s+)?(?:struct\s+)?\w[\w\s*]*?)\s+(\w+)\s*(?:\[([^\]]*)\])?\s*(?:=\s*(.+))?\s*;$"
        );
        if let Some(decl) = declaration.captures(line) {
            let first = decl[1].split(is_python_space).next().unwrap_or("");
            if !matches!(
                first,
                "if" | "else"
                    | "while"
                    | "for"
                    | "return"
                    | "switch"
                    | "break"
                    | "continue"
                    | "goto"
                    | "case"
                    | "default"
            ) {
                let c_type = strip(&decl[1]);
                let name = strip(&decl[2]);
                let array = decl.get(3).map_or("", |m| m.as_str());
                let value = decl.get(4).map_or("0", |m| m.as_str());
                let utype = if array.is_empty() {
                    map_type(c_type)
                } else {
                    map_array_type(c_type, array)
                };
                lines.push(format!("{pad}· {name} ─ {utype} ─ {} ◇", conv_expr(value)));
                continue;
            }
        }
        if !line.contains("==") && !line.starts_with("//") {
            if let Some(assignment) = regex!(r"^(.+?)\s*([+\-*/%&|^]?=)\s*(.+)\s*;$").captures(line)
            {
                lines.push(format_assignment(&assignment, &pad));
                continue;
            }
        }
        if line.ends_with(';') {
            let call = conv_expr(strip(line.trim_end_matches(';')));
            lines.push(format!("{pad}{} ◇", conv_call_expr(&call)));
            continue;
        }
        if line == "break;" {
            lines.push(format!("{pad}→ ◇  /* break */"));
            continue;
        }
        if line == "continue;" {
            lines.push(format!("{pad}/* continue */"));
            continue;
        }
        lines.push(format!("{pad}/* {line} */"));
    }
    lines
}

pub fn collect_block<'a>(lines: &[&'a str], mut i: usize) -> (Vec<&'a str>, usize) {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut started = false;
    while let Some(raw_line) = lines.get(i) {
        let line = strip(raw_line);
        i += 1;
        if line.is_empty() {
            continue;
        }
        if !started {
            if let Some(inner) = line.strip_prefix('{') {
                started = true;
                depth = 1;
                let trimmed = strip(inner);
                if !trimmed.is_empty() && trimmed != "}" {
                    result.push(trimmed);
                }
                if inner.contains('}') {
                    break;
                }
                continue;
            }
            result.push(line);
            break;
        }
        for ch in line.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            let close_line = strip(line.trim_end_matches('}'));
            if !close_line.is_empty() {
                result.push(close_line);
            }
            break;
        }
        result.push(line);
    }
    (result, i)
}

pub fn conv_local_decl(init: &str, pad: &str) -> Vec<String> {
    let declaration = regex!(r"^([\w\s*]+?)\s+(\w+)\s*(?:=\s*(.+))?$");
    if let Some(parts) = declaration.captures(strip(init).trim_end_matches(';')) {
        let c_type = map_type(strip(&parts[1]));
        let name = strip(&parts[2]);
        let value = parts.get(3).map_or("0", |m| m.as_str());
        return vec![format!("{pad}· {name} ─ {c_type} ─ {} ◇", conv_expr(value))];
    }
    conv_assignment(init.trim_end_matches(';'), pad)
}

fn format_assignment(parts: &regex::Captures<'_>, pad: &str) -> String {
    let lhs = conv_expr(&parts[1]);
    let op = &parts[2];
    let rhs = conv_expr(&parts[3]);
    if op == "=" {
        format!("{pad}{lhs} ─ {rhs} ◇")
    } else {
        let arith_op = op.chars().next().unwrap();
        format!("{pad}{lhs} ─ {lhs} {arith_op} {rhs} ◇")
    }
}

pub fn conv_assignment(stmt: &str, pad: &str) -> Vec<String> {
    let stmt = strip(stmt).trim_end_matches(';');
    if let Some(parts) = regex!(r"^(.+?)\s*([+\-*/%]?=)\s*(.+)").captures(stmt) {
        vec![format_assignment(&parts, pad)]
    } else {
        vec![format!("{pad}/* {stmt} */ ◇")]
    }
}

pub fn conv_call_expr(expr: &str) -> String {
    let Some(parts) = regex!(r"^(\w[\w.─△]*)\s*\(([^)]*)\)$").captures(strip(expr)) else {
        return expr.to_owned();
    };
    let name = &parts[1];
    let args = strip(&parts[2]);
    if args.is_empty() {
        return format!("{name} ○");
    }
    let args = args
        .split(',')
        .map(conv_expr)
        .collect::<Vec<_>>()
        .join(" ─ ");
    format!("{name} ○ {args}")
}

pub fn conv_switch(body: &str, sw_expr: &str, indent: usize, pad: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut first = true;
    let next_indent = indent + 1;
    for case in regex!(r"\bcase\b|\bdefault\b").split(body) {
        let case = strip(case);
        if case.is_empty() {
            continue;
        }
        let Some(parts) = regex!(r"(?s)^(.+?):\s*(.*)").captures(case) else {
            continue;
        };
        let value = strip(&parts[1]);
        let stmts = strip(strip(&parts[2]).trim_end_matches('}'));
        let stmts = regex!(r"\bbreak\s*;").replace_all(stmts, "");
        let prefix = if first { "Φ" } else { "▽ Φ" };
        if value.is_empty() {
            lines.push(format!("{pad}▽ ═╗"));
        } else {
            lines.push(format!("{pad}{prefix} {sw_expr} == {value} ═╗"));
        }
        lines.extend(conv_statements(&stmts, next_indent, None, "INT64"));
        lines.push(format!("{pad}╚═══ {next_indent} ═══╝"));
        first = false;
    }
    lines
}

pub fn emit_function(func: &CFunction, gv: &GVector) -> Vec<String> {
    let k = kappa(gv);
    let tc = tessa_class(k);
    let &(g1, g2, g3, g4) = gv;
    let name = &func.name;
    let mut lines = vec![format!(
        "/* ── {name} ── κ={k:.3} TESSA={tc} g=({g1:?},{g2:?},{g3:?},{g4:?}) */"
    )];
    if func.is_boot {
        lines.push("/* BOOT_CODE */".to_owned());
    }

    let mut param_parts = Vec::new();
    let mut param_types = HashMap::new();
    for param in &func.params {
        let utype = if param.is_array {
            map_array_type(&param.c_type, &param.array_size)
        } else {
            map_type(&param.c_type)
        };
        param_parts.push(format!("{} ─ {utype}", param.name));
        param_types.insert(param.name.clone(), utype);
    }
    let ret_utype = map_type(&func.return_type);
    param_parts.push(ret_utype.clone());
    lines.push(format!("△ {name} ○ {} ═╗", param_parts.join(" ─ ")));

    set_current_func(name);
    let body_lines = conv_statements(&func.body, 1, Some(&param_types), &ret_utype);
    if body_lines.is_empty() {
        lines.push("║  /* empty */".to_owned());
    } else {
        lines.extend(body_lines);
    }
    lines.push("╚═══ 1 ═══╝".to_owned());
    lines.push(String::new());
    lines
}

pub fn emit_struct(c_struct: &CStruct) -> Vec<String> {
    let mut lines = vec![format!("□ {} ═╗", c_struct.name)];
    for (name, c_type, array) in &c_struct.fields {
        let utype = if array.is_empty() {
            map_type(c_type)
        } else {
            map_array_type(c_type, array)
        };
        lines.push(format!("║  {name} ─ {utype}"));
    }
    lines.push(format!("╚═══ {} ═══╝", c_struct.fields.len().max(1)));
    lines.push(String::new());
    lines
}

pub fn emit_global(var: &CVar) -> io::Result<Vec<String>> {
    let name = &var.name;
    let utype = if var.array_size.is_empty() {
        map_type(&var.c_type)
    } else {
        map_array_type(&var.c_type, &var.array_size)
    };
    let v = if var.value.is_empty() {
        "0"
    } else {
        strip(&var.value)
    };
    if let Some(aggregate) = v.strip_prefix('{') {
        let end = aggregate
            .rfind('}')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "substring not found"))?;
        let inner = strip(&aggregate[..end]).trim_end_matches(',');
        let split = split_top(inner, ',');
        let items: Vec<_> = split
            .iter()
            .map(|item| strip(item))
            .filter(|item| !item.is_empty())
            .collect();
        let mut lines = vec![format!("· {name} ─ {utype} ─ {{")];
        if items.iter().any(|item| item.starts_with('{')) {
            for (i, item) in items.iter().enumerate() {
                let item = item
                    .split(is_python_space)
                    .filter(|word| !word.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let comma = if i + 1 < items.len() { "," } else { "" };
                lines.push(format!("║  {item}{comma}"));
            }
        } else {
            for (i, chunk) in items.chunks(8).enumerate() {
                let comma = if (i + 1) * 8 < items.len() { "," } else { "" };
                lines.push(format!("║  {}{comma}", chunk.join(", ")));
            }
        }
        lines.push("║  } ◇".to_owned());
        return Ok(lines);
    }
    Ok(vec![format!("· {name} ─ {utype} ─ {} ◇", conv_expr(v))])
}

pub fn emit_define(define: &CDefine) -> Option<String> {
    let name = &define.name;
    let mut val = strip(&define.value);
    let utype = if regex!(r"^-?\d+$").is_match(val) {
        "INT32"
    } else if regex!(r"^0[xX][0-9a-fA-F]+$").is_match(val) {
        "UINT32"
    } else if regex!(r"^-?\d+\.\d+").is_match(val) {
        "FLOAT"
    } else if matches!(val, "1" | "0" | "true" | "false") {
        val = if matches!(val, "1" | "true") {
            "1"
        } else {
            "0"
        };
        "BOOL"
    } else if regex!(r#"^".*"$"#).is_match(val) {
        "CHAR[256]"
    } else {
        return Some(format!("/* DEFINE {name} = {val} */"));
    };
    Some(format!("· {name} ─ {utype} ─ {val} ◇"))
}

// POSIX splitext preserves directory prefixes and leading dots in the final component.
fn without_extension(path: &str) -> &str {
    let start = path.rfind('/').map_or(0, |index| index + 1);
    if let Some(dot) = path[start..].rfind('.') {
        let end = start + dot;
        if path[start..end].bytes().any(|byte| byte != b'.') {
            return &path[..end];
        }
    }
    path
}

pub fn emit_uagl(unit: &CUnit, _source_path: &str) -> io::Result<String> {
    let filename = &unit.filename;
    let gv = get_gvector(filename);
    let (g1, g2, g3, g4) = gv;
    let k = kappa(&gv);
    let tc = tessa_class(k);
    let module_name = without_extension(filename)
        .to_uppercase()
        .replace(['-', '.'], "_");

    let mut lines = vec![
        format!("/* UAGL v3 — {filename}"),
        "   Source: seL4 kernel".to_owned(),
        format!("   G-Vector: g1={g1:?} g2={g2:?} g3={g3:?} g4={g4:?}"),
        format!("   κ={k:.4}  TESSA={tc}"),
        "   Converted by seL4→UAGL pipeline v1.0".to_owned(),
        "*/".to_owned(),
        String::new(),
        "#PRAGMA BIOS64".to_owned(),
        "#PRAGMA FREESTANDING".to_owned(),
        "#PRAGMA OPTIMIZE 3".to_owned(),
        String::new(),
    ];

    for include in &unit.includes {
        let include_name = without_extension(&include.path).replace(['/', '-'], "_");
        lines.push(format!("/* #INCLUDE {include_name} */"));
    }
    if !unit.includes.is_empty() {
        lines.push(String::new());
    }

    lines.push(format!("╔═ {module_name} ═╗"));
    lines.push(String::new());

    if !unit.defines.is_empty() {
        lines.push("/* ── CONSTANTS ── */".to_owned());
        lines.extend(unit.defines.iter().filter_map(emit_define));
        lines.push(String::new());
    }
    if !unit.structs.is_empty() {
        lines.push("/* ── STRUCTURES ── */".to_owned());
        lines.extend(unit.structs.iter().flat_map(emit_struct));
    }
    if !unit.globals.is_empty() {
        lines.push("/* ── GLOBALS ── */".to_owned());
        for global in &unit.globals {
            lines.extend(emit_global(global)?);
        }
        lines.push(String::new());
    }
    if !unit.functions.is_empty() {
        lines.push("/* ── FUNCTIONS ── */".to_owned());
        lines.extend(
            unit.functions
                .iter()
                .flat_map(|func| emit_function(func, &gv)),
        );
    }

    lines.push("╚═══ 1 ═══╝".to_owned());
    lines.push(String::new());
    lines.push(format!("/* 𐰉𐰆𐰕𐰴𐰆𐰺𐱃 · 𐰚𐰺𐰞 · UAGL 3.0 · {module_name} */"));
    Ok(lines.join("\n"))
}
