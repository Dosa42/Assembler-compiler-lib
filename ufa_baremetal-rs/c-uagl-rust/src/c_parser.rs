//! Regex-based C source parser, preserving the Python converter's input model.

use regex::Regex;
use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CParam {
    pub name: String,
    pub c_type: String,
    pub is_array: bool,
    pub array_size: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CFunction {
    pub name: String,
    pub return_type: String,
    pub params: Vec<CParam>,
    pub body: String,
    pub is_static: bool,
    pub is_boot: bool,
    pub line_no: usize,
    pub doc: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CStruct {
    pub name: String,
    pub fields: Vec<(String, String, String)>,
    pub is_typedef: bool,
    pub line_no: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CVar {
    pub name: String,
    pub c_type: String,
    pub value: String,
    pub is_static: bool,
    pub is_global: bool,
    pub array_size: String,
    pub line_no: usize,
}

impl Default for CVar {
    fn default() -> Self {
        Self {
            name: String::new(),
            c_type: String::new(),
            value: String::new(),
            is_static: false,
            is_global: true,
            array_size: String::new(),
            line_no: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CInclude {
    pub path: String,
    pub is_system: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CDefine {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CUnit {
    pub filename: String,
    pub includes: Vec<CInclude>,
    pub defines: Vec<CDefine>,
    pub structs: Vec<CStruct>,
    pub functions: Vec<CFunction>,
    pub globals: Vec<CVar>,
    pub raw_lines: Vec<String>,
}

fn py_regex(pattern: &str) -> Regex {
    let pattern = pattern
        .replace(r"\w", r"[\p{L}\p{N}_]")
        .replace(r"\s", r"[\s\x1c-\x1f]");
    Regex::new(&pattern).expect("valid C parser regex")
}

macro_rules! re {
    ($pattern:expr) => {{
        static REGEX: OnceLock<Regex> = OnceLock::new();
        REGEX.get_or_init(|| py_regex($pattern))
    }};
}

fn py_space(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
}

fn py_trim(value: &str) -> &str {
    value.trim_matches(py_space)
}

fn py_word(character: char) -> bool {
    let mut buffer = [0; 4];
    re!(r"^\w$").is_match(character.encode_utf8(&mut buffer))
}

pub fn strip_comments(src: &str) -> String {
    let src = re!(r"(?s)/\*.*?\*/").replace_all(src, |caps: &regex::Captures| {
        "\n".repeat(caps[0].bytes().filter(|&byte| byte == b'\n').count())
    });
    re!(r"//[^\n]*").replace_all(&src, "").into_owned()
}

pub fn strip_ifdefs(src: &str) -> String {
    src.split('\n')
        .map(|line| {
            let stripped = py_trim(line);
            if stripped.starts_with("#if")
                || stripped.starts_with("#else")
                || stripped.starts_with("#endif")
            {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn match_paren(src: &str, start: usize) -> Option<usize> {
    let mut depth = 0isize;
    for (offset, byte) in src.as_bytes()[start..].iter().copied().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_args(src: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0isize;
    let mut start = 0;
    for (index, character) in src.char_indices() {
        match character {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                args.push(py_trim(&src[start..index]));
                start = index + 1;
            }
            _ => {}
        }
    }
    args.push(py_trim(&src[start..]));
    args
}

fn replacement_error(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

// Python interprets a macro argument as a re.sub replacement template.
fn replacement_for(argument: &str, parameter: &str) -> io::Result<String> {
    let mut chars = argument.chars().peekable();
    let mut result = String::new();
    while let Some(character) = chars.next() {
        if character != '\\' {
            result.push(character);
            continue;
        }
        let escaped = chars
            .next()
            .ok_or_else(|| replacement_error("bad escape (end of pattern)"))?;
        match escaped {
            '\\' => result.push('\\'),
            'a' => result.push('\x07'),
            'b' => result.push('\x08'),
            'f' => result.push('\x0c'),
            'n' => result.push('\n'),
            'r' => result.push('\r'),
            't' => result.push('\t'),
            'v' => result.push('\x0b'),
            'g' => {
                if chars.next() != Some('<') {
                    return Err(replacement_error("missing <"));
                }
                let mut group = String::new();
                loop {
                    match chars.next() {
                        Some('>') => break,
                        Some(character) => group.push(character),
                        None => return Err(replacement_error("missing >, unterminated name")),
                    }
                }
                if group.parse::<usize>() == Ok(0) {
                    result.push_str(parameter);
                } else {
                    return Err(replacement_error(format!(
                        "unknown or invalid group reference {group}"
                    )));
                }
            }
            digit @ '0'..='9' => {
                let second = chars.peek().copied();
                let third = chars.clone().nth(1);
                let is_octal = |c| matches!(c, Some('0'..='7'));
                if digit == '0' || (digit <= '7' && is_octal(second) && is_octal(third)) {
                    let mut value = digit.to_digit(8).expect("octal digit");
                    for _ in 0..2 {
                        if !is_octal(chars.peek().copied()) {
                            break;
                        }
                        value = value * 8
                            + chars
                                .next()
                                .and_then(|c| c.to_digit(8))
                                .expect("octal digit");
                    }
                    if value > 0xff {
                        return Err(replacement_error(
                            "octal escape value outside of range 0–0o377",
                        ));
                    }
                    result.push(char::from_u32(value).expect("byte as Unicode character"));
                } else {
                    let mut group = digit.to_string();
                    if second.is_some_and(|c| c.is_ascii_digit()) {
                        group.push(chars.next().expect("second digit"));
                    }
                    return Err(replacement_error(format!(
                        "invalid group reference {group}"
                    )));
                }
            }
            letter if letter.is_ascii_alphabetic() => {
                return Err(replacement_error(format!("bad escape \\{letter}")));
            }
            other => {
                result.push('\\');
                result.push(other);
            }
        }
    }
    Ok(result)
}

fn substitute_parameter(body: &str, parameter: &str, argument: &str) -> io::Result<String> {
    let replacement = replacement_for(argument, parameter)?;
    let ascii_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut result = String::new();
    let mut copied = 0;
    let mut search = 0;
    while let Some(relative) = body[search..].find(parameter) {
        let start = search + relative;
        let end = start + parameter.len();
        let preceded = body[..start].chars().next_back().is_some_and(ascii_word);
        let followed = body[end..].chars().next().is_some_and(ascii_word);
        if preceded || followed {
            search = start
                + body[start..]
                    .chars()
                    .next()
                    .expect("nonempty parameter")
                    .len_utf8();
            continue;
        }
        result.push_str(&body[copied..start]);
        result.push_str(&replacement);
        copied = end;
        search = end;
    }
    result.push_str(&body[copied..]);
    Ok(result)
}

struct Macro {
    name: String,
    parameters: Vec<String>,
    body: String,
    call: Regex,
}

pub fn expand_function_macros(src: &str) -> io::Result<String> {
    let lines: Vec<_> = src.split('\n').collect();
    let mut out = Vec::new();
    let mut macros: Vec<Macro> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if let Some(caps) = re!(r"^\s*#\s*define\s+(\w+)\(([^)]*)\)\s*(.*)$").captures(line) {
            let name = caps[1].to_owned();
            let parameters = caps[2]
                .split(',')
                .map(py_trim)
                .filter(|param| !param.is_empty())
                .map(str::to_owned)
                .collect();
            let mut body = caps[3].to_owned();
            while body.trim_end_matches(py_space).ends_with('\\') && index + 1 < lines.len() {
                index += 1;
                body.truncate(body.trim_end_matches(py_space).len() - 1);
                body.push(' ');
                body.push_str(py_trim(lines[index]));
            }
            if let Some(previous) = macros.iter_mut().find(|definition| definition.name == name) {
                previous.parameters = parameters;
                previous.body = body;
            } else {
                let pattern = format!(r"{}\s*\(", regex::escape(&name));
                macros.push(Macro {
                    name,
                    parameters,
                    body,
                    call: py_regex(&pattern),
                });
            }
            out.push("");
        } else {
            out.push(line);
        }
        index += 1;
    }

    let mut text = out.join("\n");
    for _ in 0..8 {
        let mut changed = false;
        for definition in &macros {
            let mut search = 0;
            while let Some(found) = definition.call.find_at(&text, search) {
                let start = found.start();
                if text[..start].chars().next_back().is_some_and(py_word) {
                    search = start
                        + text[start..]
                            .chars()
                            .next()
                            .expect("nonempty macro")
                            .len_utf8();
                    continue;
                }
                let Some(end) = match_paren(&text, found.end() - 1) else {
                    break;
                };
                let args = split_args(&text[found.end()..end]);
                let mut body = definition.body.clone();
                for (parameter, arg) in definition.parameters.iter().zip(args) {
                    body = substitute_parameter(&body, parameter, arg)?;
                }
                body = body.replace("##", "");
                text.replace_range(start..end + 1, &body);
                search = start + body.len();
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Ok(text)
}

fn strip_attributes(src: &str) -> String {
    let pattern = re!(concat!(
        r"BOOT_CODE|BOOT_BSS|BOOT_RODATA|NORETURN|CONST|PURE|VISIBLE|",
        r"FORCE_INLINE|NO_INLINE|WEAK|PACKED|UNUSED|FASTPATH_CODE|",
        r"compile_assert\s*\([^)]*\)|likely|unlikely"
    ));
    let mut out = String::new();
    let mut copied = 0;
    let mut search = 0;
    while let Some(found) = pattern.find_at(src, search) {
        let start = found.start();
        let end = found.end();
        let word_before = src[..start].chars().next_back().is_some_and(py_word);
        let word_after = src[end..].chars().next().is_some_and(py_word);
        let last_is_word = found.as_str().chars().next_back().is_some_and(py_word);
        if word_before || word_after == last_is_word {
            search = start + 1;
            continue;
        }
        out.push_str(&src[copied..start]);
        copied = end;
        search = end;
    }
    out.push_str(&src[copied..]);
    out
}

pub fn preprocess(src: &str) -> io::Result<String> {
    let stripped = strip_comments(src);
    let expanded = expand_function_macros(&stripped)?;
    let normalized = re!(r"NODE_STATE\s*\(\s*(\w+)\s*\)").replace_all(&expanded, "$1");
    let without_attributes = strip_attributes(&normalized);
    Ok(re!(r"\n{3,}")
        .replace_all(&without_attributes, "\n\n")
        .into_owned())
}

pub fn parse_includes(src: &str) -> Vec<CInclude> {
    re!(r#"(?m)^\s*#\s*include\s*([<"])([^>"]+)[>"]"#)
        .captures_iter(src)
        .map(|caps| CInclude {
            path: caps[2].to_owned(),
            is_system: &caps[1] == "<",
        })
        .collect()
}

pub fn parse_defines(src: &str) -> Vec<CDefine> {
    re!(r"(?m)^\s*#\s*define\s+(\w+)(?:\s+(.+))?$")
        .captures_iter(src)
        .filter_map(|caps| {
            let name = &caps[1];
            let value = caps.get(2).map_or("", |value| py_trim(value.as_str()));
            if name.contains('(')
                || name.ends_with("_H")
                || name.ends_with("_H_")
                || value.is_empty()
            {
                return None;
            }
            Some(CDefine {
                name: name.to_owned(),
                value: value.to_owned(),
            })
        })
        .collect()
}

pub fn parse_fields(body: &str) -> Vec<(String, String, String)> {
    re!(r"(?m)^\s*(.+?)\s+(\w+)(?:\s*\[([^\]]+)\])?\s*;")
        .captures_iter(body)
        .filter_map(|caps| {
            let c_type = py_trim(&caps[1]);
            let name = py_trim(&caps[2]);
            let array_size = caps.get(3).map_or("", |size| size.as_str());
            if c_type.contains(':') || name.contains(':') || matches!(c_type, "struct" | "union") {
                return None;
            }
            Some((name.to_owned(), c_type.to_owned(), array_size.to_owned()))
        })
        .collect()
}

pub fn parse_structs(src: &str) -> Vec<CStruct> {
    let mut structs = Vec::new();
    let mut seen = HashSet::new();
    for caps in re!(r"(?s)typedef\s+struct\s*(\w*)\s*\{([^}]*)\}\s*(\w+)\s*;").captures_iter(src) {
        let name = py_trim(&caps[3]).to_owned();
        if seen.insert(name.clone()) {
            structs.push(CStruct {
                name,
                fields: parse_fields(&caps[2]),
                is_typedef: true,
                line_no: 0,
            });
        }
    }
    let plain = re!(r"(?s)struct\s+(\w+)\s*\{([^}]*)\}");
    let mut search = 0;
    while let Some(caps) = plain.captures_at(src, search) {
        let entire = caps.get(0).expect("full struct match");
        if src[..entire.start()]
            .chars()
            .next_back()
            .is_some_and(py_word)
        {
            search = entire.start() + 1;
            continue;
        }
        search = entire.end();
        let name = py_trim(&caps[1]).to_owned();
        if !name.is_empty() && seen.insert(name.clone()) {
            structs.push(CStruct {
                name,
                fields: parse_fields(&caps[2]),
                is_typedef: false,
                line_no: 0,
            });
        }
    }
    structs
}

pub fn extract_body(src: &str, start: usize) -> (&str, usize) {
    let mut depth = 0isize;
    let mut body_start = start;
    for (offset, byte) in src.as_bytes()[start..].iter().copied().enumerate() {
        let index = start + offset;
        match byte {
            b'{' => {
                depth += 1;
                if depth == 1 {
                    body_start = index + 1;
                }
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (&src[body_start..index], index + 1);
                }
            }
            _ => {}
        }
    }
    (&src[body_start..], src.len())
}

pub fn parse_params(param_str: &str) -> Vec<CParam> {
    if matches!(py_trim(param_str), "" | "void") {
        return Vec::new();
    }
    let mut params = Vec::new();
    for param in param_str.split(',').map(py_trim) {
        if matches!(param, "" | "...") {
            continue;
        }
        if let Some(caps) = re!(r"^(.+?)\s+(\w+)\s*\[([^\]]*)\]$").captures(param) {
            params.push(CParam {
                name: caps[2].to_owned(),
                c_type: py_trim(&caps[1]).to_owned(),
                is_array: true,
                array_size: caps[3].to_owned(),
            });
        } else if let Some(caps) = re!(r"^(.+?)\s*\*+\s*(\w+)$").captures(param) {
            params.push(CParam {
                name: caps[2].to_owned(),
                c_type: format!("{} *", py_trim(&caps[1])),
                ..CParam::default()
            });
        } else if let Some(split) = param.rfind(py_space) {
            params.push(CParam {
                name: param[split..].trim_start_matches(py_space).to_owned(),
                c_type: param[..split].trim_end_matches(py_space).to_owned(),
                ..CParam::default()
            });
        } else {
            params.push(CParam {
                name: param.to_owned(),
                c_type: "int".to_owned(),
                ..CParam::default()
            });
        }
    }
    params
}

pub fn parse_functions(src: &str) -> Vec<CFunction> {
    let pattern = re!(concat!(
        r"(?s)(?:static\s+|inline\s+|BOOT_CODE\s+)*",
        r"((?:const\s+)?(?:struct\s+)?\w[\w\s\*]*?)\s+",
        r"(\w+)\s*\(([^)]*)\)\s*(?:NORETURN\s+|VISIBLE\s+)*\{"
    ));
    let mut functions = Vec::new();
    for caps in pattern.captures_iter(src) {
        let return_type = py_trim(&caps[1]);
        let name = py_trim(&caps[2]);
        if matches!(
            name,
            "if" | "else"
                | "while"
                | "for"
                | "switch"
                | "do"
                | "struct"
                | "union"
                | "enum"
                | "typedef"
        ) || matches!(return_type, "" | "typedef" | "struct" | "enum" | "union")
        {
            continue;
        }
        let entire = caps.get(0).expect("full function match");
        let (body, _) = extract_body(src, entire.end() - 1);
        functions.push(CFunction {
            name: name.to_owned(),
            return_type: return_type.to_owned(),
            params: parse_params(py_trim(&caps[3])),
            body: body.to_owned(),
            is_static: entire.as_str().contains("static"),
            is_boot: entire.as_str().contains("BOOT_CODE") || name.to_lowercase().contains("boot"),
            line_no: src[..entire.start()]
                .bytes()
                .filter(|&byte| byte == b'\n')
                .count()
                + 1,
            doc: String::new(),
        });
    }
    functions
}

fn balanced(src: &str, start: usize) -> usize {
    let mut depth = 0isize;
    for (offset, byte) in src.as_bytes()[start..].iter().copied().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return start + offset;
                }
            }
            _ => {}
        }
    }
    src.char_indices()
        .next_back()
        .map_or(0, |(offset, _)| offset)
}

fn starts_with_type_declaration(head: &str) -> bool {
    let head = head.trim_start_matches(py_space);
    ["typedef", "struct", "union", "enum"]
        .into_iter()
        .any(|keyword| {
            head.strip_prefix(keyword)
                .is_some_and(|after| !after.chars().next().is_some_and(py_word))
        })
}

pub fn parse_globals(src: &str) -> Vec<CVar> {
    let src = src
        .split('\n')
        .map(|line| {
            if line.trim_start_matches(py_space).starts_with('#') {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut chunks = Vec::new();
    let mut paren_depth = 0isize;
    let mut index = 0;
    let mut start = 0;
    let last_char = src
        .char_indices()
        .next_back()
        .map_or(0, |(offset, _)| offset);
    while index < src.len() {
        let character = src[index..].chars().next().expect("valid UTF-8 boundary");
        match character {
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            '{' if paren_depth == 0 => {
                let head = &src[start..index];
                let end = balanced(&src, index);
                if head.contains('=') {
                    let semicolon = src[end..]
                        .find(';')
                        .map_or(last_char, |offset| end + offset);
                    chunks.push((start, &src[start..semicolon]));
                    index = semicolon;
                } else if starts_with_type_declaration(head) {
                    index = src[end..]
                        .find(';')
                        .map_or(last_char, |offset| end + offset);
                } else {
                    index = end;
                }
                start = index
                    + src[index..]
                        .chars()
                        .next()
                        .expect("at least opening brace")
                        .len_utf8();
            }
            ';' if paren_depth == 0 => {
                chunks.push((start, &src[start..index]));
                start = index + 1;
            }
            _ => {}
        }
        index += src[index..]
            .chars()
            .next()
            .expect("valid UTF-8 boundary")
            .len_utf8();
    }

    let pattern = re!(concat!(
        r"(?s)^(?:static\s+|extern\s+)?",
        r"((?:const\s+)?(?:struct\s+)?\w[\w\s]*?)\s+",
        r"(\w+)(?:\s*\[([^\]]*)\])?\s*(?:=\s*(.+?))?\s*$"
    ));
    let mut globals = Vec::new();
    for (position, chunk) in chunks {
        let chunk = py_trim(chunk);
        if chunk.is_empty()
            || chunk
                .split('=')
                .next()
                .is_some_and(|head| head.contains('('))
        {
            continue;
        }
        let Some(caps) = pattern.captures(chunk) else {
            continue;
        };
        let c_type = py_trim(&caps[1]);
        let name = py_trim(&caps[2]);
        if matches!(
            name,
            "if" | "else"
                | "while"
                | "for"
                | "switch"
                | "do"
                | "return"
                | "struct"
                | "union"
                | "enum"
                | "typedef"
        ) || matches!(c_type, "return" | "typedef" | "")
            || c_type.contains("typedef")
            || c_type.chars().count() > 60
        {
            continue;
        }
        let array_size = caps.get(3).map_or("", |size| size.as_str()).to_owned();
        let value = caps
            .get(4)
            .map_or("0", |value| value.as_str())
            .split(py_space)
            .filter(|piece| !piece.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        globals.push(CVar {
            name: name.to_owned(),
            c_type: c_type.to_owned(),
            value,
            is_static: chunk
                .split(name)
                .next()
                .is_some_and(|head| head.contains("static")),
            is_global: true,
            array_size,
            line_no: src[..position]
                .bytes()
                .filter(|&byte| byte == b'\n')
                .count()
                + 1,
        });
    }
    globals
}

pub fn parse_source(raw: &str, filename: &str) -> io::Result<CUnit> {
    // Python opens files in universal-newline mode before splitting raw lines.
    let raw = raw.replace("\r\n", "\n").replace('\r', "\n");
    let includes = parse_includes(&raw);
    let defines = parse_defines(&raw);
    let preprocessed = preprocess(&raw)?;
    Ok(CUnit {
        filename: filename.to_owned(),
        includes,
        defines,
        structs: parse_structs(&preprocessed),
        functions: parse_functions(&preprocessed),
        globals: parse_globals(&preprocessed),
        raw_lines: raw.split('\n').map(str::to_owned).collect(),
    })
}

pub fn parse_file(path: impl AsRef<Path>) -> io::Result<CUnit> {
    let path = path.as_ref();
    let raw = std::fs::read(path)?;
    let filename = path.file_name().unwrap_or_default().to_string_lossy();
    parse_source(&String::from_utf8_lossy(&raw), &filename)
}
