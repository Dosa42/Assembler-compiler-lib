use crate::type_map::map_type;
use regex::{Captures, Regex};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

macro_rules! re {
    ($pattern:expr) => {{
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            // Python's Unicode word and whitespace classes differ from Rust regex's.
            let pattern = $pattern
                .replace(r"\w", r"[\p{L}\p{N}_]")
                .replace(r"\s", r"[\s\x{1c}-\x{1f}]");
            Regex::new(&pattern).unwrap()
        });
        &*RE
    }};
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub id: usize,
    pub function: String,
    pub kind: String,
    pub severity: String,
    pub c: String,
    pub note: String,
}

thread_local! {
    static GAP_STATE: RefCell<(String, Vec<Gap>)> = const { RefCell::new((String::new(), Vec::new())) };
}

pub fn clear_gaps() {
    GAP_STATE.with_borrow_mut(|state| state.1.clear());
}

pub fn take_gaps() -> Vec<Gap> {
    GAP_STATE.with_borrow_mut(|state| std::mem::take(&mut state.1))
}

pub fn gaps() -> Vec<Gap> {
    GAP_STATE.with_borrow(|state| state.1.clone())
}

pub fn set_current_func(name: &str) {
    GAP_STATE.with_borrow_mut(|state| name.clone_into(&mut state.0));
}

fn record_gap(kind: &str, text: &str, note: &str, severity: &str) -> usize {
    GAP_STATE.with_borrow_mut(|(function, gaps)| {
        let id = gaps.len() + 1;
        gaps.push(Gap {
            id,
            function: function.clone(),
            kind: kind.into(),
            severity: severity.into(),
            c: collapse_space(text).chars().take(240).collect(),
            note: note.into(),
        });
        id
    })
}

fn is_space(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '\u{1c}'..='\u{1f}')
}

fn trim(text: &str) -> &str {
    text.trim_matches(is_space)
}

fn collapse_space(text: &str) -> String {
    text.split(is_space)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_letter(ch: char) -> bool {
    ch.is_ascii_alphabetic()
        || (!ch.is_ascii() && re!(r"^\p{L}$").is_match(ch.encode_utf8(&mut [0; 4])))
}

fn is_word(ch: char) -> bool {
    ch == '_' || is_letter(ch) || ch.is_numeric()
}

fn preceding(text: &str, index: usize) -> Option<char> {
    text[..index].chars().next_back()
}

fn match_close(text: &str, start: usize) -> Option<usize> {
    let open = text.as_bytes()[start];
    let close = match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        _ => unreachable!(),
    };
    let mut depth = 0;
    for (index, &byte) in text.as_bytes().iter().enumerate().skip(start) {
        if byte == open {
            depth += 1;
        } else if byte == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

pub fn split_top(text: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if ch == sep && depth == 0 {
            parts.push(text[start..index].to_owned());
            start = index + ch.len_utf8();
        }
    }
    parts.push(text[start..].to_owned());
    parts
}

fn find_top(text: &str, needle: char) -> Option<usize> {
    let mut depth = 0;
    for (index, ch) in text.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ if ch == needle && depth == 0 => return Some(index),
            _ => {}
        }
    }
    None
}

fn utype(base: &str, stars: &str, array: Option<&str>) -> String {
    let cleaned = contextual_sub(
        base,
        re!(r"const|volatile|static|register|restrict"),
        |_, before, after| {
            (!before.chars().next_back().is_some_and(is_word)
                && !after.chars().next().is_some_and(is_word))
            .then(String::new)
        },
    );
    let base = collapse_space(&cleaned);
    let special = match base.as_str() {
        "__int128_t" => Some("INT128"),
        "__uint128_t" => Some("UINT128"),
        "unsigned long long" => Some("UINT64"),
        "long long" => Some("INT64"),
        "short" => Some("INT16"),
        "signed char" => Some("INT8"),
        _ => None,
    };
    let mapped = if let Some(special) = special {
        format!("{}{special}", if stars.is_empty() { "" } else { "@" })
    } else if stars.is_empty() {
        map_type(&base)
    } else {
        map_type(&format!("{base} {}", "*".repeat(stars.len())))
    };
    if let Some(array) = array {
        let mapped = if stars.is_empty() {
            mapped.strip_prefix('@').unwrap_or(&mapped)
        } else {
            &mapped
        };
        format!("{mapped}[{array}]")
    } else {
        mapped
    }
}

fn cast(text: &str) -> Option<Captures<'_>> {
    re!(r"^\s*((?:(?:const|volatile|unsigned|signed|struct)\s+)*(?:u?int(?:8|16|32|64)_t|__u?int128_t|size_t|intptr_t|uintptr_t|char|short|long|int|void|bool|float|double)(?:\s+(?:int|long))?)\s*(\*{0,2})\s*$").captures(text)
}

fn is_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "if" | "else"
            | "for"
            | "while"
            | "do"
            | "switch"
            | "case"
            | "default"
            | "return"
            | "break"
            | "continue"
            | "goto"
            | "sizeof"
    )
}

// Each call covers one of the Python regexes that has a context assertion.
fn contextual_sub(
    text: &str,
    pattern: &Regex,
    mut substitute: impl FnMut(&Captures<'_>, &str, &str) -> Option<String>,
) -> String {
    let mut result = String::new();
    let mut copied = 0;
    let mut search = 0;
    while let Some(captures) = pattern.captures_at(text, search) {
        let full = captures.get(0).unwrap();
        if let Some(replacement) = substitute(&captures, &text[..full.start()], &text[full.end()..])
        {
            result.push_str(&text[copied..full.start()]);
            result.push_str(&replacement);
            copied = full.end();
            search = copied;
        } else {
            search = full.start() + text[full.start()..].chars().next().unwrap().len_utf8();
        }
    }
    result.push_str(&text[copied..]);
    result
}

pub fn cx(text: &str) -> String {
    let text = contextual_sub(text, re!(r"ULL|LLU|UL|LU|LL|U|L"), |_, before, after| {
        (before
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_ascii_hexdigit())
            && !after.chars().next().is_some_and(is_word))
        .then(String::new)
    });
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        let ch = text[i..].chars().next().unwrap();
        if (is_letter(ch) || ch == '_') && !preceding(&text, i).is_some_and(is_word) {
            let rest = &text[i..];
            let end = rest.find(|ch| !is_word(ch)).unwrap_or(rest.len()) + i;
            let ident = &text[i..end];
            let args_start = text.len() - text[end..].trim_start_matches([' ', '\t', '\n']).len();
            if text.as_bytes().get(args_start) == Some(&b'(') {
                if let Some(close) = match_close(&text, args_start) {
                    let inner = &text[args_start + 1..close];
                    if !is_keyword(ident) {
                        let args = split_top(inner, ',')
                            .iter()
                            .map(|arg| trim(arg))
                            .filter(|arg| !arg.is_empty())
                            .map(cx)
                            .collect::<Vec<_>>();
                        if args.is_empty() {
                            out.push_str(&format!("{ident} ○ ◇"));
                        } else {
                            out.push_str(&format!("{ident} ○ {} ◇", args.join(" ─ ")));
                        }
                        i = close + 1;
                        continue;
                    }
                    if ident == "sizeof" {
                        let inner = trim(inner);
                        let converted = match cast(inner) {
                            Some(m) => utype(&m[1], &m[2], None),
                            None => cx(inner),
                        };
                        out.push_str(&format!("SIZEOF ○ {converted} ◇"));
                        i = close + 1;
                        continue;
                    }
                }
            }
            out.push_str(ident);
            i = end;
        } else if let Some(close) = matches!(ch, '(' | '[')
            .then(|| match_close(&text, i))
            .flatten()
        {
            let inner = &text[i + 1..close];
            out.push(ch);
            if ch == '(' {
                let after_operand = out[..out.len() - 1]
                    .trim_end_matches(is_space)
                    .chars()
                    .next_back()
                    .is_some_and(|ch| is_word(ch) || matches!(ch, ']' | ')'));
                match cast(inner) {
                    Some(m) if !after_operand => out.push_str(&utype(&m[1], &m[2], None)),
                    _ => out.push_str(&cx(inner)),
                }
                out.push(')');
            } else {
                out.push_str(&cx(inner));
                out.push(']');
            }
            i = close + 1;
        } else {
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    loop {
        let replaced = re!(r"(\w+(?:\[[^\]]*\])?)\s*->\s*(\w+)").replace_all(&out, "${1} ─ △ ${2}");
        if replaced == out {
            break;
        }
        out = replaced.into_owned();
    }
    out = contextual_sub(
        &out,
        re!(r"(\w+(?:\[[^\]]*\])?)\.([A-Za-z_]\w*)"),
        |m, before, _| {
            if before.ends_with('.') || re!(r"\d$").is_match(before) {
                None
            } else {
                Some(format!("{} ─ △ {}", &m[1], &m[2]))
            }
        },
    );
    out = out.replace("!=", "╱").replace("&&", "∧").replace("||", "∨");
    let out = re!(r"!\s*(\([^()]*\)|\w+(?:\[[^\]]*\])?)").replace_all(&out, "(${1} == 0)");
    collapse_space(&out.replace("true", "1").replace("false", "0"))
}

fn cond_expr(text: &str) -> String {
    if matches!(trim(text), "1" | "true") {
        return "1".into();
    }
    let probe = text.replace("->", "").replace("<<", "").replace(">>", "");
    let converted = cx(text);
    if re!(r"(==|!=|<=|>=|<|>|&&|\|\||!)").is_match(&probe) {
        converted
    } else if re!(r"^[A-Za-z_]\w*$").is_match(&converted) {
        format!("{converted} ╱ 0")
    } else {
        format!("({converted}) ╱ 0")
    }
}

type IncDec = (String, char);

fn extract_incdec(text: &str) -> (String, Vec<IncDec>, Vec<IncDec>) {
    let mut pre = Vec::new();
    let mut post = Vec::new();
    let text = contextual_sub(text, re!(r"(\+\+|--)\s*([A-Za-z_]\w*)"), |m, _, after| {
        if after
            .trim_start_matches(is_space)
            .starts_with(['[', '.', '-', '('])
        {
            return None;
        }
        pre.push((m[2].to_owned(), m[1].chars().next().unwrap()));
        Some(m[2].to_owned())
    });
    let text = contextual_sub(
        &text,
        re!(r"([A-Za-z_]\w*)\s*(\+\+|--)"),
        |m, before, after| {
            if before.chars().next_back().is_some_and(is_word)
                || after.chars().next().is_some_and(is_word)
            {
                return None;
            }
            post.push((m[1].to_owned(), m[2].chars().next().unwrap()));
            Some(m[1].to_owned())
        },
    );
    (text, pre, post)
}

fn find_assign(text: &str) -> Option<(&str, &str, &str)> {
    let mut depth = 0;
    for (i, ch) in text.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '=' if depth == 0 && !text[i + 1..].starts_with('=') => {
                let prev = preceding(text, i);
                if matches!(prev, None | Some('=' | '!')) {
                    continue;
                }
                let lhs = &text[..i];
                let rhs = &text[i + 1..];
                if lhs.ends_with("<<") || lhs.ends_with(">>") {
                    return Some((&lhs[..i - 2], &lhs[i - 2..], rhs));
                }
                if matches!(prev, Some('<' | '>')) {
                    continue;
                }
                if matches!(prev, Some('+' | '-' | '*' | '/' | '%' | '&' | '|' | '^')) {
                    return Some((&lhs[..i - 1], &lhs[i - 1..], rhs));
                }
                return Some((lhs, "=", rhs));
            }
            _ => {}
        }
    }
    None
}

fn split_ternary(text: &str) -> Option<(&str, &str, &str)> {
    let question = find_top(text, '?')?;
    let rest = &text[question + 1..];
    let colon = find_top(rest, ':')?;
    Some((
        trim(&text[..question]),
        trim(&rest[..colon]),
        trim(&rest[colon + 1..]),
    ))
}

enum Node<'a> {
    Block(Vec<Node<'a>>),
    Empty,
    Simple(&'a str),
    If(&'a str, Option<Box<Node<'a>>>, Option<Box<Node<'a>>>),
    While(&'a str, Option<Box<Node<'a>>>),
    For(String, String, String, Option<Box<Node<'a>>>),
    Do(&'a str, Option<Box<Node<'a>>>),
    Switch(&'a str),
}

fn captures_at<'a>(pattern: &Regex, text: &'a str, start: usize) -> Option<Captures<'a>> {
    if start > text.len() {
        return None;
    }
    pattern
        .captures_at(text, start)
        .filter(|m| m.get(0).unwrap().start() == start)
}

fn strip_keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    text.strip_prefix(keyword)
        .filter(|after| !after.chars().next().is_some_and(is_word))
}

fn parse_stmt(text: &str, start: usize) -> (Option<Node<'_>>, usize) {
    if start >= text.len() {
        return (None, start);
    }
    let rest = text[start..].trim_start_matches([' ', '\t', '\r', '\n']);
    let start = text.len() - rest.len();
    let Some(first) = rest.as_bytes().first() else {
        return (None, start);
    };
    if *first == b'{' {
        let close = match_close(text, start).unwrap_or(text.len());
        return (
            Some(Node::Block(parse_list(&text[start + 1..close]))),
            close + 1,
        );
    }
    if *first == b';' {
        return (Some(Node::Empty), start + 1);
    }
    if let Some(m) = captures_at(re!(r"(if|while|for|switch)\s*\("), text, start) {
        let paren = m.get(0).unwrap().end() - 1;
        let close = match_close(text, paren).expect("maximum recursion depth exceeded");
        let head = &text[paren + 1..close];
        let (body, mut next) = parse_stmt(text, close + 1);
        let body = body.map(Box::new);
        let node = match &m[1] {
            "if" => {
                let mut otherwise = None;
                if next <= text.len() {
                    let rest = text[next..].trim_start_matches([' ', '\t', '\r', '\n']);
                    let else_start = text.len() - rest.len();
                    if strip_keyword(rest, "else").is_some() {
                        let (branch, end) = parse_stmt(text, else_start + 4);
                        otherwise = branch.map(Box::new);
                        next = end;
                    }
                }
                Node::If(head, body, otherwise)
            }
            "while" => Node::While(head, body),
            "switch" => Node::Switch(head),
            "for" => {
                let mut parts = split_top(head, ';').into_iter();
                Node::For(
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default(),
                    body,
                )
            }
            _ => unreachable!(),
        };
        return (Some(node), next);
    }
    if strip_keyword(rest, "do").is_some() {
        let (body, next) = parse_stmt(text, start + 2);
        if let Some(m) = captures_at(re!(r"\s*while\s*\("), text, next) {
            let paren = m.get(0).unwrap().end() - 1;
            let close = match_close(text, paren).expect("maximum recursion depth exceeded");
            let rest = text[close + 1..].trim_start_matches([' ', '\t', '\r', '\n', ';']);
            return (
                Some(Node::Do(&text[paren + 1..close], body.map(Box::new))),
                text.len() - rest.len(),
            );
        }
        return (Some(Node::Block(body.into_iter().collect())), next);
    }
    let end = find_top(rest, ';').map(|i| start + i).unwrap_or(text.len());
    (Some(Node::Simple(trim(&text[start..end]))), end + 1)
}

fn parse_list(text: &str) -> Vec<Node<'_>> {
    let mut nodes = Vec::new();
    let mut start = 0;
    loop {
        let (node, next) = parse_stmt(text, start);
        match node {
            Some(node) => nodes.push(node),
            None => return nodes,
        }
        start = next;
    }
}

struct Conv<'a> {
    lines: Vec<String>,
    loops: Vec<Vec<String>>,
    declared: HashSet<String>,
    types: HashMap<String, String>,
    ret_type: &'a str,
    ntmp: usize,
    scopes: Vec<HashMap<String, String>>,
}

impl Conv<'_> {
    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    fn ren(&self, text: &str) -> String {
        if self.scopes.iter().all(HashMap::is_empty) {
            return text.to_owned();
        }
        contextual_sub(text, re!(r"\w+"), |m, before, _| {
            if before
                .chars()
                .next_back()
                .is_some_and(|ch| is_word(ch) || matches!(ch, '.' | '>'))
            {
                return None;
            }
            self.scopes
                .iter()
                .rev()
                .find_map(|scope| scope.get(&m[0]).cloned())
        })
    }

    fn new_name(&mut self, name: &str) -> String {
        let mut new = name.to_owned();
        let scope = self.scopes.last_mut().unwrap();
        if self.declared.contains(name) {
            let mut k = 2;
            new = format!("{name}_{k}");
            while self.declared.contains(&new) {
                k += 1;
                new = format!("{name}_{k}");
            }
            scope.insert(name.to_owned(), new.clone());
        } else {
            scope.remove(name);
        }
        self.declared.insert(new.clone());
        new
    }

    fn put(&mut self, indent: usize, text: &str) {
        self.lines.push(format!("{}{text}", "║  ".repeat(indent)));
    }

    fn untr(&mut self, indent: usize, text: &str, kind: &str) {
        let id = record_gap(kind, text, "no deterministic translation rule", "needs-ai");
        self.put(
            indent,
            &format!("/* GAP#{id} {kind}: {} */", collapse_space(text)),
        );
    }

    fn hoist(&mut self, text: &str, indent: usize, kind: Option<&str>, loop_cond: bool) -> String {
        let mut text = text.to_owned();
        for _ in 0..8 {
            let mut depth = 0;
            let question = text.char_indices().find_map(|(i, ch)| {
                match ch {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    '?' if depth >= 1 => return Some(i),
                    _ => {}
                }
                None
            });
            let Some(question) = question else {
                break;
            };
            depth = 0;
            let open = text[..question].char_indices().rev().find_map(|(i, ch)| {
                match ch {
                    ')' => depth += 1,
                    '(' if depth == 0 => return Some(i),
                    '(' => depth -= 1,
                    _ => {}
                }
                None
            });
            let ternary = open.and_then(|open| {
                let close = match_close(&text, open)?;
                let (cond, yes, no) = split_ternary(&text[open + 1..close])?;
                Some((open, close, cond, yes, no))
            });
            let Some((open, close, cond, yes, no)) = ternary.filter(|_| !loop_cond) else {
                self.untr(indent, &text, "TERNARY_IN_EXPR");
                break;
            };
            self.ntmp += 1;
            let mut name = format!("_t{}", self.ntmp);
            while self.declared.contains(&name) {
                self.ntmp += 1;
                name = format!("_t{}", self.ntmp);
            }
            self.declared.insert(name.clone());
            let scalar = match kind {
                Some(
                    kind @ ("UINT8" | "UINT16" | "UINT32" | "UINT64" | "INT8" | "INT16" | "INT32"
                    | "INT64" | "INT128" | "UINT128" | "CHAR" | "BOOL"),
                ) => {
                    record_gap(
                        "TERNARY_HOISTED",
                        &text,
                        &format!("hoisted to {name}:{kind}; verify promotion semantics"),
                        "info",
                    );
                    kind
                }
                _ => {
                    let id = record_gap(
                        "TERNARY_TEMP_TYPE_ASSUMED",
                        &text,
                        "destination type unknown; INT64 assumed",
                        "review",
                    );
                    self.put(
                        indent,
                        &format!("/* GAP#{id} TERNARY_TEMP_TYPE_ASSUMED INT64 */"),
                    );
                    "INT64"
                }
            };
            self.put(indent, &format!("· {name} ─ {scalar} ─ 0 ◇"));
            self.emit_ternary_assign(
                |conv, expr, depth| conv.emit_simple(&format!("{name} = {expr}"), depth),
                cond,
                yes,
                no,
                indent,
            );
            text.replace_range(open..=close, &name);
        }
        text
    }

    fn emit_nodes(&mut self, nodes: &[Node<'_>], indent: usize) {
        for node in nodes {
            self.emit_node(node, indent);
        }
    }

    fn emit_body(&mut self, body: Option<&Node<'_>>, indent: usize) {
        self.push();
        match body {
            Some(Node::Block(nodes)) => self.emit_nodes(nodes, indent),
            Some(node) => self.emit_node(node, indent),
            None => {}
        }
        self.pop();
    }

    fn close(&mut self, indent: usize) {
        self.put(indent, &format!("╚═══ {} ═══╝", indent + 1));
    }

    fn emit_node(&mut self, node: &Node<'_>, indent: usize) {
        match node {
            Node::Empty => {}
            Node::Block(nodes) => {
                self.push();
                self.emit_nodes(nodes, indent);
                self.pop();
            }
            Node::Simple(text) => self.emit_simple(text, indent),
            Node::If(cond, then, otherwise) => {
                self.emit_if(cond, then.as_deref(), otherwise.as_deref(), indent, false)
            }
            Node::While(cond, body) => self.emit_loop(cond, &[], body.as_deref(), indent),
            Node::For(init, cond, incr, body) => {
                self.push();
                for stmt in split_top(init, ',') {
                    let stmt = trim(&stmt);
                    if !stmt.is_empty() {
                        self.emit_simple(stmt, indent);
                    }
                }
                let incr = split_top(incr, ',')
                    .into_iter()
                    .filter_map(|text| {
                        let text = trim(&text);
                        (!text.is_empty()).then(|| text.to_owned())
                    })
                    .collect::<Vec<_>>();
                self.emit_loop(
                    if trim(cond).is_empty() { "1" } else { cond },
                    &incr,
                    body.as_deref(),
                    indent,
                );
                self.pop();
            }
            Node::Do(cond, body) => {
                self.put(indent, "ⁿ 1 ═╗");
                self.emit_body(body.as_deref(), indent + 1);
                self.put(
                    indent + 1,
                    &format!("Φ {} ═╗", cond_expr(&format!("!({})", self.ren(cond)))),
                );
                self.put(indent + 2, "→ ◇  /* break */");
                self.close(indent + 1);
                self.close(indent);
            }
            Node::Switch(cond) => self.untr(indent, &format!("switch ({cond})"), "UNTRANSLATED"),
        }
    }

    fn emit_if(
        &mut self,
        cond: &str,
        then: Option<&Node<'_>>,
        otherwise: Option<&Node<'_>>,
        indent: usize,
        is_else_if: bool,
    ) {
        let (text, pre, post) = extract_incdec(cond);
        for (var, op) in &pre {
            self.emit_incdec(var, *op, indent);
        }
        let prefix = if is_else_if { "▽ Φ" } else { "Φ" };
        let text = self.hoist(&self.ren(&text), indent, Some("INT32"), false);
        self.put(indent, &format!("{prefix} {} ═╗", cond_expr(&text)));
        for (var, op) in &post {
            self.emit_incdec(var, *op, indent + 1);
        }
        self.emit_body(then, indent + 1);
        self.close(indent);
        match otherwise {
            Some(Node::If(cond, then, otherwise)) => {
                self.emit_if(cond, then.as_deref(), otherwise.as_deref(), indent, true)
            }
            Some(otherwise) => {
                self.put(indent, "▽ ═╗");
                self.emit_body(Some(otherwise), indent + 1);
                self.close(indent);
            }
            None => {}
        }
    }

    fn emit_incdec(&mut self, var: &str, op: char, indent: usize) {
        let var = self.ren(var);
        self.put(
            indent,
            &format!("{var} ─ {var} {} 1 ◇", if op == '+' { '+' } else { '-' }),
        );
    }

    fn emit_loop(&mut self, cond: &str, incrs: &[String], body: Option<&Node<'_>>, indent: usize) {
        let (text, pre, post) = extract_incdec(cond);
        for (var, op) in &pre {
            self.emit_incdec(var, *op, indent);
        }
        let text = self.hoist(&self.ren(&text), indent, Some("INT32"), true);
        self.put(indent, &format!("ⁿ {} ═╗", cond_expr(&text)));
        for (var, op) in &post {
            self.emit_incdec(var, *op, indent + 1);
        }
        self.loops.push(incrs.to_vec());
        self.emit_body(body, indent + 1);
        self.loops.pop();
        for stmt in incrs {
            self.emit_simple(stmt, indent + 1);
        }
        for (var, op) in &pre {
            self.emit_incdec(var, *op, indent + 1);
        }
        self.close(indent);
        for (var, op) in &post {
            self.emit_incdec(var, *op, indent);
        }
    }

    fn emit_ternary_assign(
        &mut self,
        mut emit: impl FnMut(&mut Self, &str, usize),
        cond: &str,
        yes: &str,
        no: &str,
        indent: usize,
    ) {
        self.put(indent, &format!("Φ {} ═╗", cond_expr(cond)));
        emit(self, yes, indent + 1);
        self.close(indent);
        self.put(indent, "▽ ═╗");
        emit(self, no, indent + 1);
        self.close(indent);
    }

    fn emit_simple(&mut self, text: &str, indent: usize) {
        let text = trim(text);
        if text.is_empty() || text.starts_with('#') {
            return;
        }
        if let Some(expr) = strip_keyword(text, "return") {
            let expr = trim(expr);
            if expr.is_empty() {
                self.put(indent, "→ ◇");
                return;
            }
            let (expr, pre, _) = extract_incdec(expr);
            for (var, op) in pre {
                self.emit_incdec(&var, op, indent);
            }
            let expr = self.ren(&expr);
            let expr = if split_ternary(&expr).is_none() {
                self.hoist(&expr, indent, Some(self.ret_type), false)
            } else {
                expr
            };
            if let Some((cond, yes, no)) = split_ternary(&expr) {
                self.emit_ternary_assign(
                    |conv, expr, depth| conv.put(depth, &end(format!("→ {}", cx(expr)))),
                    cond,
                    yes,
                    no,
                    indent,
                );
            } else {
                self.put(indent, &end(format!("→ {}", cx(&expr))));
            }
            return;
        }
        match text {
            "break" => {
                self.put(indent, "→ ◇  /* break */");
                return;
            }
            "continue" => {
                if let Some(incrs) = self.loops.last().cloned() {
                    for stmt in incrs {
                        self.emit_simple(&stmt, indent);
                    }
                }
                self.put(indent, "/* continue */");
                return;
            }
            _ => {}
        }
        if text.starts_with("goto ") || re!(r"^\w+:$").is_match(text) {
            self.untr(indent, text, "UNTRANSLATED");
            return;
        }
        if re!(r"^\(\s*void\s*\)\s*\w+$").is_match(text) {
            self.put(indent, &format!("/* no-op {text} */"));
            return;
        }

        let (text, pre, post) = extract_incdec(text);
        for (var, op) in &pre {
            self.emit_incdec(var, *op, indent);
        }
        if (pre.is_empty() && post.is_empty())
            || !re!(r"^\*?\s*[A-Za-z_]\w*$").is_match(trim(&text))
        {
            self.emit_core(&text, indent);
        }
        for (var, op) in &post {
            self.emit_incdec(var, *op, indent);
        }
    }

    fn emit_core(&mut self, text: &str, indent: usize) {
        if self.try_decl(text, indent) {
            return;
        }
        let text = self.ren(text);
        if let Some(m) = re!(r"^(.+?)(\+\+|--)$").captures(&text) {
            let value = cx(trim(&m[1]));
            let op = m[2].chars().next().unwrap();
            self.put(indent, &format!("{value} ─ {value} {op} 1 ◇"));
            return;
        }
        if let Some((lhs, op, rhs)) = find_assign(&text) {
            let lhs = trim(lhs);
            let mut rhs = trim(rhs);
            if let Some((inner, _, _)) = find_assign(rhs) {
                if rhs.starts_with(trim(inner)) {
                    self.emit_core(rhs, indent);
                    rhs = trim(inner);
                }
            }
            let emit_one = |conv: &mut Self, rhs: &str, depth| {
                let rv = cx(rhs);
                let dest = cx(lhs);
                if op == "=" {
                    conv.put(depth, &format!("{dest} ─ {rv} ◇"));
                } else {
                    let rv = if re!(r"^[\w.]+$").is_match(&rv) {
                        rv
                    } else {
                        format!("({rv})")
                    };
                    conv.put(depth, &format!("{dest} ─ {dest} {op} {rv} ◇"));
                }
            };
            if let Some((cond, yes, no)) = split_ternary(rhs) {
                self.emit_ternary_assign(emit_one, cond, yes, no, indent);
                return;
            }
            let lhs_type = re!(r"^\s*\*?\s*([A-Za-z_]\w*)\s*(\[|$)")
                .captures(lhs)
                .and_then(|m| {
                    let kind = self.types.get(&m[1])?;
                    Some(
                        if &m[2] == "[" {
                            kind.split('[').next().unwrap()
                        } else {
                            kind
                        }
                        .to_owned(),
                    )
                });
            let rhs = self.hoist(rhs, indent, lhs_type.as_deref(), false);
            emit_one(self, &rhs, indent);
            return;
        }
        let text = self.hoist(&text, indent, None, false);
        let expr = cx(&text);
        if expr.contains('○') {
            self.put(indent, &end(expr));
        } else {
            self.untr(indent, &text, "UNTRANSLATED");
        }
    }

    fn try_decl(&mut self, text: &str, indent: usize) -> bool {
        let declarators = split_top(text, ',');
        let first = &declarators[0];
        let head = &first[..find_top(first, '=').unwrap_or(first.len())];
        let head_nb = head.split('[').next().unwrap();
        let Some(m) = re!(r"^((?:[A-Za-z_]\w*[\s\*]+)+)([A-Za-z_]\w*)$").captures(trim(head_nb))
        else {
            return false;
        };
        let words = &m[1];
        let name = &m[2];
        if ["return", "else", "goto", "case", "typedef"]
            .iter()
            .any(|keyword| strip_keyword(trim(words), keyword).is_some())
        {
            return false;
        }
        let stars: String = words.chars().filter(|&ch| ch == '*').collect();
        let base = collapse_space(&words.replace('*', " "));
        let mut items = vec![(stars.as_str(), name, &first[head_nb.len()..])];
        for decl in &declarators[1..] {
            let Some(m) = re!(r"(?s)^\s*(\**)\s*([A-Za-z_]\w*)(.*)$").captures(decl) else {
                return false;
            };
            items.push((
                m.get(1).unwrap().as_str(),
                m.get(2).unwrap().as_str(),
                m.get(3).unwrap().as_str(),
            ));
        }
        for (stars, name, rest) in items {
            let rest = self.ren(trim(rest));
            let name = self.new_name(name);
            let (array, rest) = match re!(r"(?s)^\[([^\]]*)\]\s*(.*)$").captures(&rest) {
                Some(m) => (
                    Some(trim(m.get(1).unwrap().as_str())),
                    trim(m.get(2).unwrap().as_str()),
                ),
                None => (None, rest.as_str()),
            };
            let val = if let Some(val) = rest.strip_prefix('=') {
                Some(trim(val))
            } else if rest.is_empty() {
                None
            } else {
                return false;
            };
            let kind = utype(&base, stars, array);
            self.types.insert(name.clone(), kind.clone());
            match val {
                None => self.put(indent, &format!("· {name} ─ {kind} ─ 0 ◇")),
                Some(val) if val.starts_with('{') => {
                    self.put(indent, &format!("· {name} ─ {kind} ─ {} ◇", cx(val)))
                }
                Some(val) => {
                    if let Some((cond, yes, no)) = split_ternary(val) {
                        self.put(indent, &format!("· {name} ─ {kind} ─ 0 ◇"));
                        self.emit_ternary_assign(
                            |conv, expr, depth| {
                                conv.put(depth, &format!("{name} ─ {} ◇", cx(expr)))
                            },
                            cond,
                            yes,
                            no,
                            indent,
                        );
                    } else {
                        let val = self.hoist(val, indent, Some(&kind), false);
                        self.put(indent, &format!("· {name} ─ {kind} ─ {} ◇", cx(&val)));
                    }
                }
            }
        }
        true
    }
}

fn end(mut text: String) -> String {
    if !text.ends_with('◇') {
        text.push_str(" ◇");
    }
    text
}

fn lint_residual(lines: &[String]) {
    let patterns = [
        ("RESIDUAL_C_TERNARY", re!(r"\?")),
        ("RESIDUAL_C_SEMICOLON", re!(r";")),
        (
            "RESIDUAL_C_TYPE_NAME",
            re!(r"u?int(?:8|16|32|64)_t|__u?int128_t|size_t|unsigned|const|static"),
        ),
        ("RESIDUAL_C_INCDEC", re!(r"(?:\+\+|--)")),
        (
            "RESIDUAL_C_COMPOUND_ASSIGN",
            re!(r"(?:\+|-|\*|/|&|\||\^|<<|>>)="),
        ),
        ("RESIDUAL_C_ARROW", re!(r"->")),
        ("RESIDUAL_C_CALL_SYNTAX", re!(r"[A-Za-z_]\w*\s*\(")),
    ];
    for line in lines {
        let code = re!(r"/\*.*?\*/").replace_all(line, "");
        let code = trim(&code);
        if code.is_empty() || code.starts_with('╚') {
            continue;
        }
        if let Some((kind, _)) = patterns.iter().find(|(kind, pattern)| match *kind {
            "RESIDUAL_C_TYPE_NAME" => pattern.find_iter(code).any(|m| {
                !preceding(code, m.start()).is_some_and(is_word)
                    && !code[m.end()..].chars().next().is_some_and(is_word)
            }),
            "RESIDUAL_C_CALL_SYNTAX" => pattern
                .find_iter(code)
                .any(|m| !preceding(code, m.start()).is_some_and(is_word)),
            _ => pattern.is_match(code),
        }) {
            record_gap(kind, code, "C syntax survived conversion", "needs-ai");
        }
    }
}

pub fn convert_body(
    body: &str,
    indent: usize,
    param_types: Option<&HashMap<String, String>>,
    ret_type: &str,
) -> Vec<String> {
    let mut skip = false;
    let filtered = body
        .split('\n')
        .filter(|line| {
            if skip || line.trim_start_matches(is_space).starts_with('#') {
                skip = line.trim_end_matches(is_space).ends_with('\\');
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let types = param_types.cloned().unwrap_or_default();
    let mut conv = Conv {
        lines: Vec::new(),
        loops: Vec::new(),
        declared: types.keys().cloned().collect(),
        types,
        ret_type,
        ntmp: 0,
        scopes: vec![HashMap::new()],
    };
    conv.emit_nodes(&parse_list(&filtered), indent);
    lint_residual(&conv.lines);
    conv.lines
}
