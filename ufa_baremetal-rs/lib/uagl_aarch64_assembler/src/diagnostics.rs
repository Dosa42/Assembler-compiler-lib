use std::cell::RefCell;

/// A source-check message associated with a one-based UAGL line number.
#[derive(Debug, PartialEq, Eq)]
pub struct SourceDiagnostic {
    pub line: usize,
    pub message: String,
}

struct Diagnostics {
    line: usize,
    messages: Vec<SourceDiagnostic>,
}

thread_local! {
    // Expression codegen is recursive. This optional collector lets the exact
    // fallback sites report diagnostics without changing their assembly output.
    static ACTIVE: RefCell<Option<Diagnostics>> = const { RefCell::new(None) };
}

pub(crate) fn collect(generate: impl FnOnce()) -> Vec<SourceDiagnostic> {
    struct ClearDiagnostics;
    impl Drop for ClearDiagnostics {
        fn drop(&mut self) {
            ACTIVE.with(|active| active.borrow_mut().take());
        }
    }

    ACTIVE.with(|active| {
        active.replace(Some(Diagnostics { line: 1, messages: Vec::new() }));
    });
    let _clear_on_unwind = ClearDiagnostics;
    generate();
    ACTIVE.with(|active| active.borrow_mut().take().unwrap().messages)
}

pub(crate) fn set_line(line: usize) {
    ACTIVE.with(|active| {
        if let Some(diagnostics) = active.borrow_mut().as_mut() {
            diagnostics.line = line;
        }
    });
}

pub(crate) fn record(message: impl FnOnce() -> String) {
    ACTIVE.with(|active| {
        if let Some(diagnostics) = active.borrow_mut().as_mut() {
            diagnostics.messages.push(SourceDiagnostic { line: diagnostics.line, message: message() });
        }
    });
}
