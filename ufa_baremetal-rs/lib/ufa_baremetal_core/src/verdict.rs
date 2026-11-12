//! Port of `verdict.py`.
//!
//! A K-check is a pure function `(s_text: &str, ctx: &Ctx) -> Vec<Verdict>`.
//! [`Verdict`] is plain data: status + diagnostic + a stable line-number
//! anchor into the input `.s` text. No randomness, no time.
//!
//! Per UFA Bare-Metal brief §2.4: every verdict is PASS, FAIL, or
//! NOT_APPLICABLE. No probabilistic checks.

use std::fmt;

/// One of the three verdict outcomes. The Python original represents this
/// as one of three string constants (`PASS`/`FAIL`/`NOT_APPLICABLE`); this
/// port uses a closed enum instead so an invalid fourth status is a
/// compile error rather than a stringly-typed runtime possibility — a
/// strictly stronger reading of the brief's "only PASS/FAIL/NOT_APPLICABLE,
/// no probabilistic scoring" non-negotiable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Status {
    Pass,
    Fail,
    NotApplicable,
}

impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::NotApplicable => "NOT_APPLICABLE",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single K-check finding: status + diagnostic + a stable line-number
/// anchor into the input `.s` text.
///
/// `k_class` is `&'static str` (always one of the string literals `"K01"`
/// through `"K06"` in practice, since every construction site in
/// [`crate::k_invariants`] passes a literal) rather than an enum, matching
/// the Python original's deliberately loose "plain data" typing — a K07
/// added later per brief §2.6 needs no enum variant added here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub status: Status,
    pub k_class: &'static str,
    pub file: Option<String>,
    pub function: Option<String>,
    pub line_no: Option<u32>,
    pub diagnostic: String,
    pub suggested_fix: String,
}

impl Verdict {
    /// Start building a verdict. Mirrors the Python dataclass's required
    /// positional fields (`status`, `k_class`); everything else defaults
    /// to "absent" the way the Python defaults (`None`, `""`) do, and is
    /// filled in with the builder methods below.
    pub fn new(status: Status, k_class: &'static str) -> Self {
        Verdict {
            status,
            k_class,
            file: None,
            function: None,
            line_no: None,
            diagnostic: String::new(),
            suggested_fix: String::new(),
        }
    }

    pub fn file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// Mirrors passing `file=ctx.get("file")` in Python, where `ctx["file"]`
    /// may itself be `None` — takes an `Option` directly instead of forcing
    /// every call site to branch.
    pub fn file_opt(mut self, file: Option<String>) -> Self {
        self.file = file;
        self
    }

    pub fn function(mut self, function: impl Into<String>) -> Self {
        self.function = Some(function.into());
        self
    }

    pub fn line_no(mut self, line_no: u32) -> Self {
        self.line_no = Some(line_no);
        self
    }

    pub fn diagnostic(mut self, diagnostic: impl Into<String>) -> Self {
        self.diagnostic = diagnostic.into();
        self
    }

    pub fn suggested_fix(mut self, suggested_fix: impl Into<String>) -> Self {
        self.suggested_fix = suggested_fix.into();
        self
    }

    pub fn is_fail(&self) -> bool {
        self.status == Status::Fail
    }

    pub fn is_pass(&self) -> bool {
        self.status == Status::Pass
    }

    /// Renders one diagnostic line, matching `Verdict.render()` in
    /// `verdict.py` exactly (including the em-dash before the diagnostic
    /// text and the `\n    fix: ` continuation for FAILs with a suggested
    /// fix).
    pub fn render(&self) -> String {
        let mut loc = self.file.clone().unwrap_or_else(|| "<input>".to_string());
        if let Some(n) = self.line_no {
            loc = format!("{loc}:{n}");
        }
        if let Some(func) = &self.function {
            if !func.is_empty() {
                loc = format!("{loc} in {func}()");
            }
        }
        let mut msg = format!("[{} {}] {}", self.k_class, self.status, loc);
        if !self.diagnostic.is_empty() {
            msg.push_str(&format!(" — {}", self.diagnostic));
        }
        if !self.suggested_fix.is_empty() && self.is_fail() {
            msg.push_str(&format!("\n    fix: {}", self.suggested_fix));
        }
        msg
    }
}

/// Mirrors `all_pass()` in `verdict.py`.
pub fn all_pass<'a>(verdicts: impl IntoIterator<Item = &'a Verdict>) -> bool {
    verdicts.into_iter().all(|v| v.status != Status::Fail)
}

/// Mirrors `filter_fails()` in `verdict.py`.
pub fn filter_fails(verdicts: &[Verdict]) -> Vec<&Verdict> {
    verdicts.iter().filter(|v| v.is_fail()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_pass_no_location_extras() {
        let v = Verdict::new(Status::Pass, "K01")
            .file("good_fn.s")
            .function("good_fn")
            .line_no(4)
            .diagnostic("frame, spill discipline, and offset alignment OK");
        assert_eq!(
            v.render(),
            "[K01 PASS] good_fn.s:4 in good_fn() — frame, spill discipline, and offset alignment OK"
        );
    }

    #[test]
    fn render_fail_with_fix_appends_continuation_line() {
        let v = Verdict::new(Status::Fail, "K01")
            .file("k01_imbalance.s")
            .function("bad_frame")
            .line_no(4)
            .diagnostic("frame imbalance: SUB SP totals 32 bytes, ADD SP totals 16 bytes")
            .suggested_fix(
                "Make sure every prologue `SUB SP, SP, #N(, LSL #12)?` is matched by an epilogue `ADD SP, SP, #N(, LSL #12)?` with the same total. See Bug #1, Bug #19, Bug #20.",
            );
        assert_eq!(
            v.render(),
            "[K01 FAIL] k01_imbalance.s:4 in bad_frame() — frame imbalance: SUB SP totals 32 bytes, ADD SP totals 16 bytes\n    fix: Make sure every prologue `SUB SP, SP, #N(, LSL #12)?` is matched by an epilogue `ADD SP, SP, #N(, LSL #12)?` with the same total. See Bug #1, Bug #19, Bug #20."
        );
    }

    #[test]
    fn render_pass_with_suggested_fix_omits_fix_line() {
        // Python: `if self.suggested_fix and self.is_fail()` -- a PASS with
        // a (theoretically) non-empty suggested_fix must NOT print it.
        let v = Verdict::new(Status::Pass, "K01").suggested_fix("should not print");
        assert!(!v.render().contains("fix:"));
    }

    #[test]
    fn render_no_file_falls_back_to_input_placeholder() {
        let v = Verdict::new(Status::NotApplicable, "K05")
            .diagnostic("K05 applies only to *_boot.s / *_init.s files; this file is not a boot-protocol handoff point.");
        assert_eq!(
            v.render(),
            "[K05 NOT_APPLICABLE] <input> — K05 applies only to *_boot.s / *_init.s files; this file is not a boot-protocol handoff point."
        );
    }

    #[test]
    fn all_pass_and_filter_fails() {
        let verdicts = vec![
            Verdict::new(Status::Pass, "K01"),
            Verdict::new(Status::Fail, "K02"),
            Verdict::new(Status::NotApplicable, "K05"),
        ];
        assert!(!all_pass(&verdicts));
        assert_eq!(filter_fails(&verdicts).len(), 1);

        let clean = vec![
            Verdict::new(Status::Pass, "K01"),
            Verdict::new(Status::NotApplicable, "K05"),
        ];
        assert!(all_pass(&clean));
        assert!(filter_fails(&clean).is_empty());
    }
}
