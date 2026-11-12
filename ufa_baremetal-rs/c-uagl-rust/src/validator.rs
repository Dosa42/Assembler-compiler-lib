//! UAGL v3 syntax validation from the original converter.

use regex::Regex;
use std::{fs, io, sync::LazyLock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    pub line_no: usize,
    pub rule: &'static str,
    pub message: String,
    pub line_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationResult {
    pub filename: String,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationError>,
    pub passed: bool,
}

impl ValidationResult {
    pub fn new(filename: &str) -> Self {
        Self {
            filename: filename.to_owned(),
            errors: Vec::new(),
            warnings: Vec::new(),
            passed: true,
        }
    }

    pub fn add_error(
        &mut self,
        line_no: usize,
        rule: &'static str,
        message: impl Into<String>,
        line_text: &str,
    ) {
        self.errors.push(ValidationError {
            line_no,
            rule,
            message: message.into(),
            line_text: line_text.to_owned(),
        });
        self.passed = false;
    }

    pub fn add_warning(
        &mut self,
        line_no: usize,
        rule: &'static str,
        message: impl Into<String>,
        line_text: &str,
    ) {
        self.warnings.push(ValidationError {
            line_no,
            rule,
            message: message.into(),
            line_text: line_text.to_owned(),
        });
    }
}

static TERMINATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"◇\s*$").unwrap());
static BLOCK_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"═╗\s*$").unwrap());
static BLOCK_CLOSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"╚═══\s*\d+\s*═══╝\s*$").unwrap());
static MODULE_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"╔═\s*\w[\w\s]*\s*═╗\s*$").unwrap());
static STRUCT_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^□\s+\w+\s*═╗\s*$").unwrap());
static COMMENT_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*/\*.*\*/\s*$").unwrap());
static PRAGMA_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*#(PRAGMA|INCLUDE)\b").unwrap());
static BLOCK_INNER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*║").unwrap());
static STRUCT_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*║\s+\w+\s*─\s*\w").unwrap());
static FUNC_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^△\s+\w+.*═╗\s*$").unwrap());

#[derive(Default)]
pub struct UaglValidator;

fn strip(text: &str) -> &str {
    text.trim_matches(|ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch))
}

fn first_60(text: &str) -> &str {
    &text[..text
        .char_indices()
        .nth(60)
        .map_or(text.len(), |(index, _)| index)]
}

impl UaglValidator {
    pub fn new() -> Self {
        Self
    }

    pub fn validate(&self, filename: &str) -> io::Result<ValidationResult> {
        let bytes = fs::read(filename)?;
        Ok(self.validate_source(&String::from_utf8_lossy(&bytes), filename))
    }

    pub fn validate_source(&self, source: &str, filename: &str) -> ValidationResult {
        let mut result = ValidationResult::new(filename);
        let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
        let lines: Vec<_> = normalized.split('\n').collect();

        if !lines.iter().any(|line| line.contains("#PRAGMA BIOS")) {
            result.add_error(0, "PRAGMA_MISSING", "No #PRAGMA BIOS* directive found", "");
        }
        if !lines.iter().any(|line| MODULE_OPEN.is_match(line)) {
            result.add_error(0, "MODULE_MISSING", "No ╔═ MODULE ═╗ block found", "");
        }
        if !lines.iter().any(|line| BLOCK_CLOSE.is_match(line)) {
            result.add_error(
                0,
                "MODULE_CLOSE_MISSING",
                "No ╚═══ N ═══╝ block close found",
                "",
            );
        }

        let mut in_struct = false;
        let mut in_function = false;
        let mut struct_depth = 0;
        for (index, line) in lines.into_iter().enumerate() {
            let no = index + 1;
            let stripped = strip(line);
            if stripped.is_empty()
                || COMMENT_LINE.is_match(stripped)
                || stripped.starts_with("/*")
                || stripped.ends_with("*/")
                || PRAGMA_LINE.is_match(stripped)
                || MODULE_OPEN.is_match(stripped)
            {
                continue;
            }

            let is_inner = BLOCK_INNER.is_match(line);
            let closes = BLOCK_CLOSE.is_match(stripped);
            let opens = BLOCK_OPEN.is_match(stripped);
            if closes && !is_inner {
                if in_struct && struct_depth <= 1 {
                    in_struct = false;
                    struct_depth = 0;
                } else if in_function {
                    in_function = false;
                }
                continue;
            }
            if STRUCT_OPEN.is_match(stripped) && !in_function {
                in_struct = true;
                struct_depth = 1;
                continue;
            }
            if FUNC_OPEN.is_match(stripped) && !is_inner {
                in_function = true;
                in_struct = false;
                continue;
            }

            if in_struct && !in_function {
                if closes {
                    struct_depth -= 1;
                    if struct_depth <= 0 {
                        in_struct = false;
                        struct_depth = 0;
                    }
                    continue;
                }
                if opens {
                    struct_depth += 1;
                    continue;
                }
                if STRUCT_FIELD.is_match(line) {
                    if stripped.contains('◇') {
                        result.add_error(
                            no,
                            "STRUCT_FIELD_TERMINATOR",
                            "Struct field must not end with ◇",
                            stripped,
                        );
                    }
                    continue;
                }
                if stripped.starts_with('║') && !stripped.starts_with("║  ") {
                    result.add_warning(
                        no,
                        "STRUCT_INDENT",
                        "Struct field indentation should be ║  (two spaces)",
                        stripped,
                    );
                    continue;
                }
            }

            if opens || closes {
                continue;
            }
            if is_inner {
                let inner = strip(line.trim_start_matches('║'));
                if !inner.is_empty()
                    && !BLOCK_OPEN.is_match(inner)
                    && !BLOCK_CLOSE.is_match(inner)
                    && !inner.starts_with("/*")
                    && !inner.starts_with('▽')
                    && !inner.starts_with('#')
                    && !TERMINATOR.is_match(inner)
                {
                    result.add_error(
                        no,
                        "MISSING_TERMINATOR",
                        format!("Statement must end with ◇: {}", first_60(inner)),
                        stripped,
                    );
                }
                continue;
            }
            if stripped.starts_with('·') {
                if !TERMINATOR.is_match(stripped) {
                    result.add_error(
                        no,
                        "MISSING_TERMINATOR",
                        "Variable declaration must end with ◇",
                        first_60(stripped),
                    );
                }
                continue;
            }
            if !stripped.ends_with('◇')
                && !stripped.starts_with("//")
                && !stripped.starts_with('#')
                && !TERMINATOR.is_match(stripped)
            {
                result.add_warning(
                    no,
                    "POSSIBLE_MISSING_TERMINATOR",
                    format!("Line may be missing ◇: {}", first_60(stripped)),
                    stripped,
                );
            }
        }
        result
    }

    pub fn validate_batch(&self, filepaths: &[String]) -> Vec<ValidationResult> {
        filepaths
            .iter()
            .map(|filename| match self.validate(filename) {
                Ok(result) => result,
                Err(error) => {
                    let mut result = ValidationResult::new(filename);
                    result.add_error(0, "PARSE_ERROR", error.to_string(), "");
                    result
                }
            })
            .collect()
    }
}

pub fn format_result(result: &ValidationResult) -> String {
    let status = if result.passed {
        "✅ PASS"
    } else {
        "❌ FAIL"
    };
    let mut lines = vec![format!("{status} {}", result.filename)];
    for error in &result.errors {
        lines.push(format!(
            "  ERROR L{} [{}]: {}",
            error.line_no, error.rule, error.message
        ));
    }
    for warning in result.warnings.iter().take(3) {
        lines.push(format!(
            "  WARN  L{} [{}]: {}",
            warning.line_no, warning.rule, warning.message
        ));
    }
    lines.join("\n")
}
