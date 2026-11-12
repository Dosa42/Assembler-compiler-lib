//! Reverse translation for differential testing of the converter's UAGL.

use regex::{Captures, Regex};
use std::{collections::HashSet, fs, path::Path, sync::LazyLock};

const TYPES: &[(&str, &str)] = &[
    ("UINT8", "uint8_t"),
    ("UINT16", "uint16_t"),
    ("UINT32", "uint32_t"),
    ("UINT64", "uint64_t"),
    ("INT8", "int8_t"),
    ("INT16", "int16_t"),
    ("INT32", "int32_t"),
    ("INT64", "int64_t"),
    ("INT128", "__int128_t"),
    ("UINT128", "__uint128_t"),
    ("CHAR", "char"),
    ("VOID", "void"),
];
const EXPORTED: &[&str] = &[
    "ed25519_publickey",
    "ed25519_sign",
    "ed25519_verify",
    "ed25519_selftest",
];

fn python_regex(pattern: &str) -> Regex {
    let pattern = pattern
        .replace(r"\w", r"[\p{L}\p{N}_]")
        .replace(r"\s", r"[\p{White_Space}\x{1C}-\x{1F}]");
    Regex::new(&pattern).expect("valid reverse-translator pattern")
}

static CAST: LazyLock<Regex> = LazyLock::new(|| {
    let names = TYPES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join("|");
    python_regex(&format!(r"\((@?(?:{names}))\)"))
});
static FIELD: LazyLock<Regex> =
    LazyLock::new(|| python_regex(r"([A-Za-z_]\w*(?:\[[^\]]*\])?) ─ △ (\w+)"));
static TERMINATOR: LazyLock<Regex> = LazyLock::new(|| python_regex(r"\s*◇\s*$"));
static DECL_ARRAY: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^(@?\w+)\[(\w+)\]$"));
static INDENT: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^(?:║\s*)+"));
static AGGREGATE: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^· (\w+) ─ ([^─]+?) ─ \{$"));
static AGGREGATE_END: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^║\s*\} ◇$"));
static AGGREGATE_INDENT: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^║\s*"));
static AGGREGATE_ARRAY: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^(\w+)\[(\d+)\]$"));
static FUNCTION: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^△ (\w+) ○ (.*) ═╗$"));
static PARAM_ARRAY: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^(\w+)\[(\w+)\]$"));
static GLOBAL: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^· (\w+) ─ ([^─]+?) ─ (.*) ◇$"));
static CONTROL: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^(▽ Φ|Φ|ⁿ) (.*) ═╗$"));
static LOCAL: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^· (\w+) ─ (.+?) ─ (.*)$"));
static CALL: LazyLock<Regex> = LazyLock::new(|| python_regex(r"^[A-Za-z_]\w* ○"));
static CALL_NAME_SUFFIX: LazyLock<Regex> = LazyLock::new(|| python_regex(r"\w*$"));

fn trim(text: &str) -> &str {
    text.trim_matches(|ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch))
}

fn type_name(ty: &str) -> &str {
    TYPES
        .iter()
        .find_map(|&(uagl, c)| (uagl == ty).then_some(c))
        .unwrap_or(ty)
}

fn ctype(ty: &str) -> String {
    let base = type_name(ty.trim_start_matches('@'));
    if ty.starts_with('@') {
        format!("{base} *")
    } else {
        base.to_owned()
    }
}

fn split_args(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let separator = " ─ ";
    for (index, ch) in text.char_indices() {
        if index < start {
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && text[index..].starts_with(separator) && !text[index..].starts_with(" ─ △")
        {
            parts.push(trim(&text[start..index]));
            start = index + separator.len();
        }
    }
    parts.push(trim(&text[start..]));
    parts
}

fn conv_calls(mut text: &str) -> String {
    let mut out = String::new();
    while let Some(open) = text.find('○') {
        let prefix = text[..open].trim_end_matches(' ');
        let name_start = CALL_NAME_SUFFIX
            .find(prefix)
            .expect("identifier suffix can be empty")
            .start();
        let name = &prefix[name_start..];
        let tail = &text[open + '○'.len_utf8()..];

        let mut depth = 1;
        let mut inner_end = 0;
        let mut call_end = tail.len();
        for (index, ch) in tail.char_indices() {
            inner_end = index;
            match ch {
                '○' => depth += 1,
                '◇' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                call_end = index + ch.len_utf8();
                break;
            }
        }
        let inner = trim(&tail[..inner_end]);
        let args: Vec<_> = if inner.is_empty() {
            Vec::new()
        } else {
            split_args(inner).into_iter().map(conv_calls).collect()
        };
        out.push_str(&text[..name_start]);
        if name == "SIZEOF" {
            out.push_str("sizeof(");
            if let Some(first) = args.first() {
                out.push_str(&ctype(first));
            }
        } else {
            out.push_str(name);
            out.push('(');
            out.push_str(&args.join(", "));
        }
        out.push(')');
        text = &tail[call_end..];
    }
    out.push_str(text);
    out
}

fn conv_expr(text: &str, pointers: &HashSet<String>) -> String {
    let calls = conv_calls(text);
    let mut result = CAST
        .replace_all(&calls, |caps: &Captures<'_>| {
            format!("({})", ctype(&caps[1]))
        })
        .into_owned();
    loop {
        let next = FIELD.replace_all(&result, |caps: &Captures<'_>| {
            let name = &caps[1];
            let base = name.split('[').next().unwrap_or_default();
            let operator = if pointers.contains(base) { "->" } else { "." };
            format!("{name}{operator}{}", &caps[2])
        });
        if next == result {
            break;
        }
        result = next.into_owned();
    }
    let result = result
        .replace('╱', "!=")
        .replace('∧', "&&")
        .replace('∨', "||");
    trim(&TERMINATOR.replace(&result, "")).to_owned()
}

fn decl(name: &str, ty: &str, init: &str, pointers: &mut HashSet<String>) -> String {
    let clean;
    let init = if init.contains('○') {
        init
    } else {
        clean = TERMINATOR.replace(init, "");
        &clean
    };
    if let Some(array) = DECL_ARRAY.captures(ty) {
        let element = ctype(&array[1]);
        let value = if init == "0" {
            "{0}".to_owned()
        } else {
            conv_expr(init, pointers)
        };
        return format!("{element} {name}[{}] = {value};", &array[2]);
    }
    if ty.starts_with('@') {
        pointers.insert(name.to_owned());
        return format!("{}{name} = {};", ctype(ty), conv_expr(init, pointers));
    }
    let converted = type_name(ty);
    let value = if converted == ty && init == "0" {
        "{0}".to_owned()
    } else {
        conv_expr(init, pointers)
    };
    format!("{converted} {name} = {value};")
}

/// Convert UAGL text to the same C text produced by `uagl2c.py`.
pub fn convert(source: &str) -> Result<String, String> {
    let normalized;
    let source = if source.contains('\r') {
        normalized = source.replace("\r\n", "\n").replace('\r', "\n");
        &normalized
    } else {
        source
    };
    let mut lines = source.split('\n');
    let mut out = vec![
        "#include <stdint.h>".to_owned(),
        "#include <string.h>".to_owned(),
        "#include \"ed25519_full.h\"".to_owned(),
        "#include \"crypto_d2.h\"".to_owned(),
        String::new(),
    ];
    let mut depth = 0;
    let mut pointers = HashSet::new();
    while let Some(raw) = lines.next() {
        let body;
        let line = if depth == 0 {
            trim(raw)
        } else {
            body = INDENT.replace(raw, "");
            trim(&body)
        };
        if line.is_empty()
            || line.starts_with("/*")
            || line.starts_with('#')
            || line.starts_with('╔')
        {
            continue;
        }
        if depth == 0 {
            if let Some(global) = AGGREGATE.captures(line) {
                let name = &global[1];
                let ty = trim(&global[2]);
                let mut rows = Vec::new();
                loop {
                    let row = lines
                        .next()
                        .ok_or_else(|| "list index out of range".to_owned())?;
                    if AGGREGATE_END.is_match(trim(row)) {
                        break;
                    }
                    rows.push(AGGREGATE_INDENT.replace(row, ""));
                }
                let body = rows.join("\n");
                if let Some(array) = AGGREGATE_ARRAY.captures(ty) {
                    out.push(format!(
                        "const {} {name}[{}] = {{\n{body}\n}};",
                        type_name(&array[1]),
                        &array[2]
                    ));
                } else {
                    out.push(format!("static const {ty} {name} = {{\n{body}\n}};"));
                }
            } else if let Some(function) = FUNCTION.captures(line) {
                let name = &function[1];
                let rest = split_args(&function[2]);
                let (ret, params) = rest
                    .split_last()
                    .expect("split_args always returns at least one field");
                pointers.clear();
                let mut converted = Vec::new();
                for pair in params.chunks(2) {
                    let param = pair[0];
                    let ty = pair
                        .get(1)
                        .ok_or_else(|| "list index out of range".to_owned())?;
                    if let Some(array) = PARAM_ARRAY.captures(ty) {
                        converted.push(format!("{} {param}[{}]", type_name(&array[1]), &array[2]));
                        pointers.insert(param.to_owned());
                    } else if ty.starts_with('@') {
                        converted.push(format!("{}{param}", ctype(ty)));
                        pointers.insert(param.to_owned());
                    } else {
                        converted.push(format!("{} {param}", type_name(ty)));
                    }
                }
                let params = if converted.is_empty() {
                    "void".to_owned()
                } else {
                    converted.join(", ")
                };
                let prefix = if EXPORTED.contains(&name) {
                    ""
                } else {
                    "static "
                };
                out.push(format!("{prefix}{} {name}({params}) {{", ctype(ret)));
                depth = 1;
            } else if let Some(global) = GLOBAL.captures(line) {
                out.push(format!(
                    "static {}",
                    decl(
                        &global[1],
                        trim(&global[2]),
                        &global[3],
                        &mut HashSet::new()
                    )
                ));
            }
            continue;
        }

        if line.starts_with("╚═══") {
            out.push("}".to_owned());
            depth -= 1;
        } else if let Some(control) = CONTROL.captures(line) {
            let keyword = match &control[1] {
                "▽ Φ" => "else if",
                "Φ" => "if",
                "ⁿ" => "while",
                _ => unreachable!("matched a control-flow keyword"),
            };
            out.push(format!(
                "{keyword} ({}) {{",
                conv_expr(&control[2], &pointers)
            ));
            depth += 1;
        } else if line == "▽ ═╗" {
            out.push("else {".to_owned());
            depth += 1;
        } else if let Some(value) = line.strip_prefix('→') {
            let value = trim(value);
            if value.starts_with('◇') {
                out.push("return;".to_owned());
            } else {
                out.push(format!("return {};", conv_expr(value, &pointers)));
            }
        } else if let Some(local) = LOCAL.captures(line) {
            out.push(decl(&local[1], trim(&local[2]), &local[3], &mut pointers));
        } else if CALL.is_match(line) {
            out.push(format!("{};", conv_expr(line, &pointers)));
        } else {
            let parts = split_args(line);
            let lhs = conv_expr(parts[0], &pointers);
            let rhs = conv_expr(&parts[1..].join(" ─ "), &pointers);
            out.push(format!("{lhs} = {rhs};"));
        }
    }
    Ok(out.join("\n") + "\n")
}

/// Read UAGL from a UTF-8 file and write the reverse translation to C.
pub fn convert_file(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<(), String> {
    let source = fs::read_to_string(src).map_err(|error| error.to_string())?;
    fs::write(dst, convert(&source)?).map_err(|error| error.to_string())
}
