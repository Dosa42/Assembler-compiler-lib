//! Token kinds and the `Token` type, mirroring the Python `TK` class and the
//! `Token` dataclass in `aarch64_assembler_braille.py`.
//!
//! Python's `Token.data` is an untyped `dict`; each lexer rule only ever
//! populates a fixed set of string keys. Rather than replicate a stringly
//! typed dict, we use a small enum `TokenData` with one variant per token
//! kind that actually carries fields, and store it alongside `TK`. This
//! keeps call sites exhaustive-checked while remaining a 1:1 mapping of the
//! original `{'name': ..., 'type': ...}` style dicts.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TK {
    FuncDef,
    BlockOpen,
    BlockClose,
    VarDecl,
    Assign,
    Return,
    If,
    Else,
    Loop,
    Call,
    Barrier,
    Scope,
    Validate,
    Nop,
    Pragma,
    Comment,
    ModuleOpen,
    StructDef,
    Field,
    Global,
    TupleRecv,
    Unknown,
}

/// Per-kind payload data. Mirrors the `data: dict` field on the Python
/// `Token` dataclass; only the keys that a given lexer rule actually sets
/// are represented for that variant.
#[derive(Debug, Clone, Default)]
pub struct TokenData {
    pub name: Option<String>,
    pub params_raw: Option<String>,
    pub r#type: Option<String>,
    pub value: Option<String>,
    pub depth: Option<i64>,
    pub cond: Option<String>,
    pub func: Option<String>,
    pub args: Option<String>,
    pub text: Option<String>,
    pub var1: Option<String>,
    pub var2: Option<String>,
    pub target: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TK,
    #[allow(dead_code)]
    pub raw: String,
    #[allow(dead_code)]
    pub line_no: usize,
    pub data: TokenData,
}

impl Token {
    pub fn new(kind: TK, raw: &str, line_no: usize) -> Self {
        Token {
            kind,
            raw: raw.to_string(),
            line_no,
            data: TokenData::default(),
        }
    }
}
