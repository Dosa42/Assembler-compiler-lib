//! Deterministic port of the fidelity.py report.

use regex::Regex;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::sync::LazyLock;

const TYPES: &str = "(?:uint8_t|uint16_t|uint32_t|uint64_t|int8_t|int16_t|int32_t|int64_t|__int128_t|__uint128_t|size_t)";
const KEYWORDS: &[&str] = &[
    "if", "else", "for", "while", "switch", "case", "return", "sizeof", "do", "goto", "break",
    "continue", "default",
];

fn python_pattern(pattern: &str) -> Regex {
    Regex::new(
        &pattern
            .replace(r"\w", r"[\p{L}\p{N}_]")
            .replace(r"\s", r"[\p{White_Space}\x{001C}-\x{001F}]"),
    )
    .expect("valid report regex")
}

macro_rules! regex {
    ($pattern:expr) => {{
        static RE: LazyLock<Regex> = LazyLock::new(|| python_pattern(&$pattern));
        &*RE
    }};
}

fn space(c: char) -> bool {
    c.is_whitespace() || ('\u{001c}'..='\u{001f}').contains(&c)
}

fn trim(text: &str) -> &str {
    text.trim_matches(space)
}

fn word(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '_';
    }
    let mut utf8 = [0; 4];
    regex!(r"^\w$").is_match(c.encode_utf8(&mut utf8))
}

fn before(text: &str, offset: usize) -> Option<char> {
    text[..offset].chars().next_back()
}

fn after(text: &str, offset: usize) -> Option<char> {
    text[offset..].chars().next()
}

fn boundary(text: &str, offset: usize) -> bool {
    before(text, offset).is_some_and(word) != after(text, offset).is_some_and(word)
}

fn balanced(text: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0;
    for (offset, &byte) in text.as_bytes()[start..].iter().enumerate() {
        if byte == open {
            depth += 1;
        } else if byte == close {
            depth -= 1;
            if depth == 0 {
                return Some(start + offset);
            }
        }
    }
    None
}

// Replacing an existing key preserves its original position, like Python dict.
fn insert<T>(entries: &mut Vec<(String, T)>, name: &str, value: T) {
    if let Some((_, previous)) = entries.iter_mut().find(|(key, _)| key == name) {
        *previous = value;
    } else {
        entries.push((name.to_owned(), value));
    }
}

type Entries = Vec<(String, String)>;
type Macros = Vec<(String, (Vec<String>, String))>;
type Counts = HashMap<String, usize>;

fn expand_macros(source: &str) -> (String, Macros) {
    let mut lines = source.split('\n');
    let mut output = Vec::new();
    let mut definitions = Vec::new();
    while let Some(line) = lines.next() {
        let stripped = line.trim_start_matches(space);
        if stripped.starts_with("#define") {
            let mut full = line.to_owned();
            while let Some(prefix) = full.trim_end_matches(space).strip_suffix('\\') {
                let Some(next) = lines.next() else { break };
                full = format!("{prefix} {next}");
            }
            if let Some(groups) = regex!(r"^\s*#define\s+(\w+)\(([^)]*)\)\s*(.*)").captures(&full) {
                let args = groups[2].split(',').map(|a| trim(a).to_owned()).collect();
                insert(&mut definitions, &groups[1], (args, groups[3].to_owned()));
            }
        } else if !stripped.starts_with('#') {
            output.push(line);
        }
    }
    (output.join("\n"), definitions)
}

// re.sub interprets backslashes in the replacement, even when it comes from
// an argument in the input program. Group 0 is the only existing group.
fn replacement(template: &str, matched: &str) -> Result<String, String> {
    let mut chars = template.chars().peekable();
    let mut output = String::new();
    while let Some(c) = chars.next() {
        if c != '\\' {
            output.push(c);
            continue;
        }
        let escape = chars
            .next()
            .ok_or("bad escape at end of macro replacement")?;
        let value = match escape {
            'a' => '\x07',
            'b' => '\x08',
            'f' => '\x0c',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\x0b',
            '\\' => '\\',
            'g' => {
                if chars.next() != Some('<') {
                    return Err("missing < in macro replacement group".into());
                }
                let mut group = String::new();
                loop {
                    match chars.next() {
                        Some('>') => break,
                        Some(c) => group.push(c),
                        None => return Err("missing > in macro replacement group".into()),
                    }
                }
                if trim(&group).parse::<i32>() != Ok(0) {
                    return Err(format!("invalid macro replacement group {group}"));
                }
                output.push_str(matched);
                continue;
            }
            '0'..='9' => {
                let lookahead: Vec<_> = chars.clone().take(2).collect();
                let octal = |c: &char| matches!(c, '0'..='7');
                if escape != '0'
                    && (!octal(&escape) || lookahead.len() != 2 || !lookahead.iter().all(octal))
                {
                    return Err(format!("invalid macro replacement group {escape}"));
                }
                let mut codepoint = escape.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    if let Some(digit) = chars.peek().and_then(|c| c.to_digit(8)) {
                        chars.next();
                        codepoint = 8 * codepoint + digit;
                    }
                }
                if codepoint > 255 {
                    return Err("octal macro replacement escape outside 0–255".into());
                }
                char::from_u32(codepoint).expect("octal byte")
            }
            c if c.is_ascii_alphabetic() => {
                return Err(format!("bad escape \\{c} in macro replacement"));
            }
            c => {
                output.push('\\');
                c
            }
        };
        output.push(value);
    }
    Ok(output)
}

fn replace_argument(text: &str, name: &str, actual: &str) -> Result<String, String> {
    let needle = Regex::new(&regex::escape(name)).expect("literal macro argument");
    let value = replacement(actual, name)?;
    let mut output = String::new();
    let mut last = 0;
    let mut search = 0;
    while let Some(found) = needle.find_at(text, search) {
        if boundary(text, found.start()) && boundary(text, found.end()) {
            output.push_str(&text[last..found.start()]);
            output.push_str(&value);
            last = found.end();
            if !found.is_empty() {
                search = last;
                continue;
            }
        }
        // A rejected candidate does not consume bytes in Python's regex.
        let Some(c) = after(text, found.start()) else {
            break;
        };
        search = found.start() + c.len_utf8();
    }
    output.push_str(&text[last..]);
    Ok(output)
}

fn expand_calls(mut body: String, definitions: &Macros) -> Result<String, String> {
    for (name, (parameters, template)) in definitions {
        let call = Regex::new(&format!(
            r"{}[\p{{White_Space}}\x{{001C}}-\x{{001F}}]*\(",
            regex::escape(name)
        ))
        .expect("valid macro call regex");
        while let Some(found) = call.find_iter(&body).find(|m| boundary(&body, m.start())) {
            let Some(end) = balanced(&body, found.end() - 1, b'(', b')') else {
                break;
            };
            let mut expanded = template.clone();
            for (parameter, actual) in parameters.iter().zip(body[found.end()..end].split(',')) {
                expanded = replace_argument(&expanded, parameter, trim(actual))?;
            }
            let expanded = expanded.replace("##", "");
            if body[found.start()..=end] == expanded {
                return Err(format!("recursive macro call {name}"));
            }
            body.replace_range(found.start()..=end, &expanded);
        }
    }
    Ok(body)
}

fn parse_c(source: &str) -> Result<(Entries, Entries), String> {
    let source = regex!(r"(?s)/\*.*?\*/").replace_all(source, " ");
    let source = regex!(r"//[^\n]*").replace_all(&source, "");
    let (source, definitions) = expand_macros(&source);
    let mut functions = Vec::new();
    let mut globals = Vec::new();
    let mut offset = 0;
    while offset < source.len() {
        let remaining = &source[offset..];
        let head_len = remaining.find([';', '{', '}']).unwrap_or(remaining.len());
        if head_len == 0 {
            offset += 1;
            continue;
        }
        let head = &remaining[..head_len];
        let end = offset + head_len;
        if source.as_bytes().get(end) == Some(&b'{') {
            let close = balanced(&source, end, b'{', b'}').ok_or("unclosed C brace")?;
            let signature = regex!(r"(\w+)\s*\([^()]*(?:\([^()]*\)[^()]*)*\)\s*$");
            if let Some(function) = signature
                .captures(trim(head))
                .filter(|_| !head.contains('='))
            {
                let body = expand_calls(source[end + 1..close].to_owned(), &definitions)?;
                insert(&mut functions, &function[1], body);
                offset = close + 1;
            } else {
                let declaration_end = source[close..].find(';').map(|i| close + i);
                if let Some(global) =
                    regex!(r"(\w+)\s*(?:\[[^\]]*\])*\s*=\s*$").captures(trim(head))
                {
                    let init_end = declaration_end.unwrap_or_else(|| {
                        source
                            .char_indices()
                            .next_back()
                            .map_or(source.len(), |(i, _)| i)
                    });
                    insert(&mut globals, &global[1], source[end..init_end].to_owned());
                }
                offset = declaration_end.unwrap_or(close) + 1;
            }
        } else {
            let signature = regex!(r"(?s)(\w+)\s*(?:\[[^\]]*\])*\s*=\s*(.*)$");
            if let Some(global) = signature
                .captures(trim(head))
                .filter(|_| !head.split('=').next().unwrap_or("").contains('('))
            {
                insert(&mut globals, &global[1], global[2].to_owned());
            }
            offset = end + 1;
        }
    }
    Ok((functions, globals))
}

fn parse_uagl(source: &str) -> Result<(Entries, Entries), String> {
    let starts = regex!(r"(?m)^△ ").find_iter(source).map(|m| m.start());
    let mut functions = Vec::new();
    let mut starts = starts.peekable();
    while let Some(start) = starts.next() {
        let end = starts.peek().copied().unwrap_or(source.len());
        let part = &source[start..end];
        let name = regex!(r"^△ (\w+)")
            .captures(part)
            .ok_or("missing UAGL function name")?;
        let body = part.split_once("\n/* ── ").map_or(part, |(body, _)| body);
        insert(&mut functions, &name[1], body.to_owned());
    }
    let mut globals = Vec::new();
    for groups in regex!(r"(?ms)^· (\w+) ─ ([^─\n]+?) ─ (\{.*?\n║  \} ◇)").captures_iter(source)
    {
        insert(&mut globals, &groups[1], groups[3].to_owned());
    }
    for groups in regex!(r"(?m)^· (\w+) ─ ([^─\n]+?) ─ ([^{\n].*?) ◇$").captures_iter(source)
    {
        if !globals.iter().any(|(name, _)| *name == groups[1]) {
            globals.push((groups[1].to_owned(), groups[3].to_owned()));
        }
    }
    Ok((functions, globals))
}

fn decimal_digit(c: char) -> Option<u32> {
    if c.is_ascii() {
        return c.to_digit(10);
    }
    fn is_decimal(c: char) -> bool {
        let mut buf = [0; 4];
        regex!(r"^\p{Nd}$").is_match(c.encode_utf8(&mut buf))
    }
    if !is_decimal(c) {
        return None;
    }
    let mut start = c as u32;
    while start > 0 && char::from_u32(start - 1).is_some_and(is_decimal) {
        start -= 1;
    }
    Some((c as u32 - start) % 10)
}

fn decimal_hex(digits: &str) -> String {
    let mut decimal = vec![0u32];
    for c in digits.chars() {
        let mut carry = c.to_digit(16).expect("hex digit");
        for digit in &mut decimal {
            carry += *digit * 16;
            *digit = carry % 10;
            carry /= 10;
        }
        while carry > 0 {
            decimal.push(carry % 10);
            carry /= 10;
        }
    }
    decimal
        .into_iter()
        .rev()
        .map(|d| char::from_digit(d, 10).unwrap())
        .collect()
}

fn literals(text: &str) -> Counts {
    let mut counts = Counts::new();
    let exclusion = |c: char| word(c) || c == '.';
    for (start, c) in text.char_indices() {
        if decimal_digit(c).is_none() || before(text, start).is_some_and(exclusion) {
            continue;
        }
        let tail = &text[start..];
        let hexadecimal = tail.strip_prefix("0x").or_else(|| tail.strip_prefix("0X"));
        if let Some(hex) = hexadecimal {
            let len = hex.bytes().take_while(u8::is_ascii_hexdigit).count();
            if len > 0 && !after(text, start + 2 + len).is_some_and(exclusion) {
                *counts.entry(decimal_hex(&hex[..len])).or_default() += 1;
            }
            continue;
        }
        let mut end = start;
        let mut digits = String::new();
        for c in tail.chars() {
            let Some(digit) = decimal_digit(c) else { break };
            digits.push(char::from_digit(digit, 10).unwrap());
            end += c.len_utf8();
        }
        if !after(text, end).is_some_and(exclusion) {
            let trimmed = digits.trim_start_matches('0');
            let normalized = if trimmed.is_empty() { "0" } else { trimmed };
            *counts.entry(normalized.to_owned()).or_default() += 1;
        }
    }
    counts
}

fn calls(text: &str, uagl: bool) -> Counts {
    let pattern = if uagl {
        regex!(r"([A-Za-z_]\w*)\s*○")
    } else {
        regex!(r"([A-Za-z_]\w*)\s*\(")
    };
    let mut counts = Counts::new();
    for groups in pattern.captures_iter(text) {
        let name = groups.get(1).unwrap();
        if before(text, name.start()).is_some_and(word) {
            continue;
        }
        let name = name.as_str();
        if uagl
            || (!KEYWORDS.contains(&name)
                && name != "void"
                && !regex!(format!("^{TYPES}$")).is_match(name))
        {
            *counts.entry(name.to_owned()).or_default() += 1;
        }
    }
    counts
}

fn matching_words(text: &str, pattern: &Regex, end_boundary: bool) -> usize {
    pattern
        .find_iter(text)
        .filter(|m| {
            !before(text, m.start()).is_some_and(word)
                && (!end_boundary || !after(text, m.end()).is_some_and(word))
        })
        .count()
}

#[derive(Default)]
struct Controls {
    loops: usize,
    conditions: usize,
    elses: usize,
    returns: usize,
}

fn c_controls(text: &str) -> Controls {
    let ternary = text.matches('?').count();
    Controls {
        loops: matching_words(text, regex!(r"(?:for|while)\s*\("), false),
        conditions: matching_words(text, regex!(r"if\s*\("), false) + ternary,
        elses: matching_words(text, regex!(r"else"), true) + ternary,
        returns: matching_words(text, regex!(r"return"), true),
    }
}

fn u_controls(text: &str) -> Controls {
    Controls {
        loops: text.matches('ⁿ').count(),
        conditions: text.matches('Φ').count(),
        elses: text.matches('▽').count(),
        returns: text.matches('→').count(),
    }
}

fn c_syntax_left(line: &str) -> bool {
    let syntax = regex!(format!(
        r";|(?:\+|-|\*|/|&|\||\^|<<|>>)=|\+\+|--|\(\s*(?:{TYPES}|int|const)[^)]*\)|\?|[{{}}]"
    ));
    if syntax.is_match(line) {
        return true;
    }
    // The keyword alternatives can overlap within a word (e.g. "uint64_t"
    // versus "int"). Splitting with Python word semantics applies both
    // boundaries to the complete word before testing its spelling.
    let word_exact = regex!(format!(
        r"^(?:{TYPES}|const|static|unsigned|int|void|struct|for|while|switch|goto)$"
    ));
    line.split(|c| !word(c))
        .any(|token| word_exact.is_match(token))
        || matching_words(line, regex!(r"[A-Za-z_]\w*\s*\("), false) > 0
}

fn count_statements(text: &str) -> usize {
    let mut depth = 0i64;
    let mut statements = 0;
    for c in text.bytes() {
        match c {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b';' if depth == 0 => statements += 1,
            _ => {}
        }
    }
    statements
}

fn missing(reference: &Counts, actual: &Counts) -> usize {
    reference
        .iter()
        .map(|(token, &count)| count.saturating_sub(actual.get(token).copied().unwrap_or(0)))
        .sum()
}

#[derive(Default)]
struct Totals {
    ok: usize,
    fail: usize,
    c_statements: usize,
    u_statements: usize,
    c_loops: usize,
    u_loops: usize,
    c_conditions: usize,
    u_conditions: usize,
    missing_literals: usize,
    all_literals: usize,
    missing_calls: usize,
    leftover: usize,
    commented_blocks: usize,
}

/// Produce the fidelity Markdown, including its final newline.
pub fn report(
    c_path: &str,
    c_source: &str,
    uagl_path: &str,
    uagl_source: &str,
) -> Result<String, String> {
    let (c_functions, c_globals) = parse_c(c_source)?;
    let (u_functions, u_globals) = parse_uagl(uagl_source)?;
    let mut totals = Totals::default();
    let mut output = format!(
        "# C → UAGL fidelity report\n\nC: `{c_path}`  UAGL: `{uagl_path}`\n\nFunctions in C (after macro expansion): {}  · in UAGL: {}\n\n## Per function\n\n| function | C stmts | UAGL stmts | loops C/U | if C/U | else C/U | ret C/U | calls missing | literals missing | C syntax left (lines) | verdict |\n|---|---|---|---|---|---|---|---|---|---|---|\n",
        c_functions.len(), u_functions.len()
    );
    for (name, c) in &c_functions {
        let Some((_, raw_uagl)) = u_functions.iter().find(|(key, _)| key == name) else {
            writeln!(output, "| {name} | - | MISSING | | | | | | | | FAIL |").unwrap();
            totals.fail += 1;
            continue;
        };
        let uagl = regex!(r"(?s)/\*.*?\*/").replace_all(raw_uagl, " ");
        let body_lines = uagl.split('\n').skip(1).filter(|line| {
            let line = trim(line);
            !line.is_empty() && !line.starts_with(['╚', '╔'])
        });
        let u_statements = body_lines.clone().filter(|line| line.contains('◇')).count();
        let c_statements = count_statements(c);
        let c_count = c_controls(c);
        let u_count = u_controls(&uagl);
        let call_missing = missing(&calls(c, false), &calls(&uagl, true));
        let c_literals = literals(c);
        let literal_missing = missing(&c_literals, &literals(&uagl));
        let literal_total: usize = c_literals.values().sum();
        let leftover = body_lines.filter(|line| c_syntax_left(line)).count();
        let commented = raw_uagl.matches("/*").count().saturating_sub(1);
        let mut issues = Vec::new();
        if (u_statements as f64) < (c_statements as f64) * 0.9 {
            issues.push("stmts");
        }
        if c_count.loops != u_count.loops {
            issues.push("loops");
        }
        if c_count.conditions != u_count.conditions {
            issues.push("if");
        }
        if call_missing != 0 {
            issues.push("calls");
        }
        if literal_missing != 0 {
            issues.push("literals");
        }
        if leftover != 0 {
            issues.push("C-syntax");
        }
        let verdict = if issues.is_empty() {
            totals.ok += 1;
            "OK".into()
        } else {
            totals.fail += 1;
            format!("FAIL: {}", issues.join(","))
        };
        totals.c_statements += c_statements;
        totals.u_statements += u_statements;
        totals.c_loops += c_count.loops;
        totals.u_loops += u_count.loops;
        totals.c_conditions += c_count.conditions;
        totals.u_conditions += u_count.conditions;
        totals.missing_literals += literal_missing;
        totals.all_literals += literal_total;
        totals.leftover += leftover;
        totals.missing_calls += call_missing;
        totals.commented_blocks += commented;
        writeln!(
            output,
            "| {name} | {c_statements} | {u_statements} | {}/{} | {}/{} | {}/{} | {}/{} | {call_missing} | {literal_missing}/{literal_total} | {leftover} | {verdict} |",
            c_count.loops, u_count.loops,
            c_count.conditions, u_count.conditions,
            c_count.elses, u_count.elses,
            c_count.returns, u_count.returns,
        ).unwrap();
    }
    output.push_str("\n## Globals / initializers\n\n| global | C literals | UAGL literals | verdict |\n|---|---|---|---|\n");
    let mut globals_failed = 0;
    for (name, init) in &c_globals {
        let c_count: usize = literals(init).values().sum();
        let uagl_init = u_globals.iter().find(|(key, _)| key == name);
        let u_count: usize = uagl_init.map_or(0, |(_, init)| literals(init).values().sum());
        let verdict = if c_count == u_count && uagl_init.is_some() {
            "OK"
        } else {
            globals_failed += 1;
            "FAIL"
        };
        writeln!(output, "| {name} | {c_count} | {u_count} | {verdict} |").unwrap();
    }
    writeln!(
        output,
        "\n## Totals\n\n- functions OK / FAIL: {} / {}\n- statements: C {} vs UAGL {}\n- loops: C {} vs UAGL {}\n- if-branches (ternary `?:` counted as if/else): C {} vs UAGL {}\n- calls in C missing from UAGL: {}\n- numeric literals in function bodies missing from UAGL: {} / {}\n- UAGL body lines still containing C syntax: {}\n- globals with lost initializers: {globals_failed} / {}\n- code blocks present only inside comments (not translated): {}\n\n## Semantic equivalence\n\nNot established. Only a token-multiset proxy (literals, calls, control-flow counts) is measured. Proving equivalence needs executing both sides on the same inputs; no UAGL executor exists in this session other than the AArch64 backend, which was out of scope.",
        totals.ok, totals.fail, totals.c_statements, totals.u_statements,
        totals.c_loops, totals.u_loops, totals.c_conditions, totals.u_conditions,
        totals.missing_calls, totals.missing_literals, totals.all_literals,
        totals.leftover, c_globals.len(), totals.commented_blocks,
    ).unwrap();
    Ok(output)
}

pub fn report_file(c_path: &str, uagl_path: &str, report_path: &str) -> Result<String, String> {
    let c_bytes = fs::read(c_path).map_err(|e| e.to_string())?;
    let c_source = String::from_utf8_lossy(&c_bytes)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let uagl_source = fs::read_to_string(uagl_path)
        .map_err(|e| e.to_string())?
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let markdown = report(c_path, &c_source, uagl_path, &uagl_source)?;
    fs::write(report_path, &markdown).map_err(|e| e.to_string())?;
    Ok(markdown)
}
