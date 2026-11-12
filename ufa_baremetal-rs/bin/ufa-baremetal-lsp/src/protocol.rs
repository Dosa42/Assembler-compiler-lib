//! LSP message dispatch and the `Verdict` -> `Diagnostic` translation.
//!
//! This is intentionally a small, fixed method set -- exactly what an
//! editor needs to get live K-check diagnostics plus hover text, not a
//! general-purpose LSP framework. Same "one file, no plugin system"
//! discipline `k_invariants.rs` itself follows (see that file's own doc
//! comment): if a K07 is added to the core registry, this file needs no
//! changes at all, since it always iterates whatever `K_REGISTRY`
//! currently holds via `audit_text`.
//!
//! Deliberately NOT implemented: `textDocument/codeAction` / auto-apply-
//! fix quick actions. Most `suggested_fix` text is prose describing what
//! to change (only K05's is an exact, literal instruction sequence) --
//! turning that into an automatic text edit for every K-check would be
//! doing ROADMAP.md item #7's (`--fix` mode) job, which that item's own
//! writeup marks as the one roadmap item with real *downside* (it can
//! corrupt a working tree) and deliberately gates behind "maturity/trust
//! in the underlying checks" first. This server surfaces `suggested_fix`
//! as read-only diagnostic/hover text instead, which carries none of
//! that risk.

use std::collections::HashMap;
use std::io::Write;

use ufa_baremetal_core::codegen_audit;
use ufa_baremetal_core::parse_s::ParseCompletenessError;
use ufa_baremetal_core::verdict::Verdict;

use crate::json_value::Value;
use crate::rpc;

fn send(out: &mut impl Write, msg: &Value) {
    // A write failure here means the client's stdin pipe is gone (it's
    // exiting or already dead) -- nothing left to signal it *to*, so
    // this is intentionally fire-and-forget rather than panicking the
    // server over a client that's already going away.
    let _ = rpc::write_message(out, &crate::json_value::to_string(msg));
}

fn send_response(out: &mut impl Write, id: Value, result: Value) {
    send(out, &Value::Obj(vec![("jsonrpc".into(), Value::str("2.0")), ("id".into(), id), ("result".into(), result)]));
}

fn send_error(out: &mut impl Write, id: Value, code: i64, message: &str) {
    send(
        out,
        &Value::Obj(vec![
            ("jsonrpc".into(), Value::str("2.0")),
            ("id".into(), id),
            (
                "error".into(),
                Value::Obj(vec![("code".into(), Value::int(code)), ("message".into(), Value::str(message))]),
            ),
        ]),
    );
}

fn send_notification(out: &mut impl Write, method: &str, params: Value) {
    send(
        out,
        &Value::Obj(vec![
            ("jsonrpc".into(), Value::str("2.0")),
            ("method".into(), Value::str(method)),
            ("params".into(), params),
        ]),
    );
}

struct Document {
    text: String,
    version: i64,
}

pub struct Server {
    docs: HashMap<String, Document>,
    initialized: bool,
}

impl Server {
    pub fn new() -> Self {
        Server { docs: HashMap::new(), initialized: false }
    }

    /// Handles one already-parsed JSON-RPC message. Returns `true` if the
    /// caller's read loop should stop (an `exit` notification arrived).
    pub fn handle(&mut self, msg: Value, out: &mut impl Write) -> bool {
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        // Per spec: nothing but `initialize` may be handled before it's
        // received. Requests get an explicit "not initialized yet" error
        // so a strict client doesn't hang; notifications are just
        // dropped (there's no id to answer on, and the client will
        // re-send state once initialized anyway via didOpen).
        if !self.initialized && method != "initialize" && method != "exit" {
            if let Some(id) = id {
                send_error(out, id, -32002, "server not initialized");
            }
            return false;
        }

        match method.as_str() {
            "initialize" => self.on_initialize(id, out),
            "initialized" => {}
            "shutdown" => {
                if let Some(id) = id {
                    send_response(out, id, Value::Null);
                }
            }
            "exit" => return true,
            "textDocument/didOpen" => self.on_did_open(&params, out),
            "textDocument/didChange" => self.on_did_change(&params, out),
            "textDocument/didSave" => self.on_did_save(&params, out),
            "textDocument/didClose" => self.on_did_close(&params, out),
            "textDocument/hover" => self.on_hover(id, &params, out),
            "$/cancelRequest" | "$/setTrace" | "workspace/didChangeConfiguration" => {}
            _ => {
                // Any other *request* (it has an id) must still get a
                // reply -- an unanswered request is how naive clients
                // hang forever. Unknown *notifications* are silently
                // ignored, per spec ("Notifications... should be
                // ignored" for methods the receiver doesn't understand).
                if let Some(id) = id {
                    send_error(out, id, -32601, &format!("method not found: {method}"));
                }
            }
        }
        false
    }

    fn on_initialize(&mut self, id: Option<Value>, out: &mut impl Write) {
        self.initialized = true;
        let capabilities = Value::Obj(vec![
            (
                "textDocumentSync".into(),
                Value::Obj(vec![
                    ("openClose".into(), Value::Bool(true)),
                    // 1 == TextDocumentSyncKind.Full. Simplest correct
                    // choice: every didChange carries the whole buffer,
                    // so there's no incremental-patch state to get wrong
                    // -- worth the extra bytes on the wire for a tool
                    // whose own non-negotiable (per the core crate's own
                    // doc comment) is correctness over cleverness.
                    ("change".into(), Value::int(1)),
                    ("save".into(), Value::Obj(vec![("includeText".into(), Value::Bool(false))])),
                ]),
            ),
            ("hoverProvider".into(), Value::Bool(true)),
        ]);
        let result = Value::Obj(vec![
            ("capabilities".into(), capabilities),
            (
                "serverInfo".into(),
                Value::Obj(vec![
                    ("name".into(), Value::str("ufa-baremetal-lsp")),
                    ("version".into(), Value::str(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
        ]);
        if let Some(id) = id {
            send_response(out, id, result);
        }
    }

    fn on_did_open(&mut self, params: &Value, out: &mut impl Write) {
        let Some(td) = params.get("textDocument") else { return };
        let Some(uri) = td.get("uri").and_then(Value::as_str) else { return };
        let text = td.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        let version = td.get("version").and_then(Value::as_i64).unwrap_or(0);
        self.docs.insert(uri.to_string(), Document { text, version });
        self.publish_diagnostics(uri, out);
    }

    fn on_did_change(&mut self, params: &Value, out: &mut impl Write) {
        let Some(td) = params.get("textDocument") else { return };
        let Some(uri) = td.get("uri").and_then(Value::as_str) else { return };
        let version = td.get("version").and_then(Value::as_i64).unwrap_or(0);
        let Some(changes) = params.get("contentChanges").and_then(Value::as_array) else { return };
        let Some(last) = changes.last() else { return };

        // We advertised Full sync only (see on_initialize), so a
        // spec-compliant client sends exactly one change event per
        // notification, carrying the entire new document text with no
        // `range`. Defensively refuse to apply anything that looks like
        // an incremental (ranged) edit instead of silently treating a
        // fragment as the whole buffer -- wrong diagnostics from stale
        // text are worse than briefly-missing diagnostics.
        if last.get("range").is_some() {
            eprintln!(
                "ufa-baremetal-lsp: got a ranged textDocument/didChange for {uri}, but only full-document \
sync was advertised in initialize; ignoring this update rather than guessing."
            );
            return;
        }
        let Some(text) = last.get("text").and_then(Value::as_str) else { return };
        self.docs.insert(uri.to_string(), Document { text: text.to_string(), version });
        self.publish_diagnostics(uri, out);
    }

    fn on_did_save(&mut self, params: &Value, out: &mut impl Write) {
        let Some(td) = params.get("textDocument") else { return };
        let Some(uri) = td.get("uri").and_then(Value::as_str) else { return };
        // We asked for `includeText: false`, so this is normally absent;
        // handled anyway in case a client sends it unprompted -- prefer
        // the freshest text it's offering over what's cached.
        if let Some(text) = params.get("text").and_then(Value::as_str) {
            let version = self.docs.get(uri).map(|d| d.version).unwrap_or(0);
            self.docs.insert(uri.to_string(), Document { text: text.to_string(), version });
        }
        self.publish_diagnostics(uri, out);
    }

    fn on_did_close(&mut self, params: &Value, out: &mut impl Write) {
        let Some(td) = params.get("textDocument") else { return };
        let Some(uri) = td.get("uri").and_then(Value::as_str) else { return };
        self.docs.remove(uri);
        // Clear whatever we last published for this buffer -- otherwise
        // closing the file leaves stale squiggles in e.g. VS Code's
        // Problems panel with nothing left to associate them with.
        send_notification(
            out,
            "textDocument/publishDiagnostics",
            Value::Obj(vec![("uri".into(), Value::str(uri)), ("diagnostics".into(), Value::Arr(vec![]))]),
        );
    }

    fn on_hover(&mut self, id: Option<Value>, params: &Value, out: &mut impl Write) {
        let Some(id) = id else { return }; // hover is always a request, never a notification
        let result = (|| -> Option<Value> {
            let uri = params.get("textDocument")?.get("uri")?.as_str()?;
            let doc = self.docs.get(uri)?;
            let line0 = params.get("position")?.get("line")?.as_i64()? as u32;
            let name = uri_to_display_name(uri);
            let verdicts = codegen_audit::audit_text(&doc.text, &name).ok()?;
            let hits: Vec<&Verdict> =
                verdicts.iter().filter(|v| v.is_fail() && v.line_no.map(|n| n.saturating_sub(1)) == Some(line0)).collect();
            if hits.is_empty() {
                return None;
            }
            let mut md = String::new();
            for (i, v) in hits.iter().enumerate() {
                if i > 0 {
                    md.push_str("\n\n---\n\n");
                }
                md.push_str(&format!("**[{} {}]** {}", v.k_class, v.status, v.diagnostic));
                if !v.suggested_fix.is_empty() {
                    md.push_str(&format!("\n\n*fix:* {}", v.suggested_fix));
                }
            }
            Some(Value::Obj(vec![(
                "contents".into(),
                Value::Obj(vec![("kind".into(), Value::str("markdown")), ("value".into(), Value::str(md))]),
            )]))
        })();
        send_response(out, id, result.unwrap_or(Value::Null));
    }

    fn publish_diagnostics(&self, uri: &str, out: &mut impl Write) {
        let Some(doc) = self.docs.get(uri) else { return };
        if !should_check(uri) {
            return; // Not a `.s` file -- nothing we understand how to check.
        }
        let name = uri_to_display_name(uri);
        let diagnostics: Vec<Value> = match codegen_audit::audit_text(&doc.text, &name) {
            Ok(verdicts) => verdicts.iter().filter(|v| v.is_fail()).map(|v| verdict_to_diagnostic(v, &doc.text)).collect(),
            Err(e) => vec![parse_error_diagnostic(&e)],
        };
        send_notification(
            out,
            "textDocument/publishDiagnostics",
            Value::Obj(vec![
                ("uri".into(), Value::str(uri)),
                ("version".into(), Value::int(doc.version)),
                ("diagnostics".into(), Value::Arr(diagnostics)),
            ]),
        );
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

/// Only `.s` files are ever meaningfully checkable -- matches
/// `codegen_audit::audit_directory`'s own `"*.s"` glob. An editor is
/// expected to only launch/route this server for that filetype in the
/// first place (see the editor configs this ships with), but this is a
/// cheap independent guard against a broader client sending us
/// everything it opens.
fn should_check(uri: &str) -> bool {
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    path.to_ascii_lowercase().ends_with(".s")
}

/// A `file://` URI's last path segment, percent-decoded -- used both as
/// the display name embedded in each verdict and as the name shown in a
/// parse-completeness error, matching what `audit_text`'s doc comment
/// says that parameter is for.
fn uri_to_display_name(uri: &str) -> String {
    let path_part = uri.strip_prefix("file://").unwrap_or(uri);
    let decoded = percent_decode(path_part);
    decoded.rsplit('/').next().unwrap_or(&decoded).to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn verdict_to_diagnostic(v: &Verdict, text: &str) -> Value {
    let line0 = v.line_no.map(|n| n.saturating_sub(1)).unwrap_or(0);
    let line_text = text.lines().nth(line0 as usize).unwrap_or("");
    let end_char = utf16_len(line_text);

    let mut message = String::new();
    if let Some(func) = &v.function {
        if !func.is_empty() {
            message.push_str(&format!("in {func}(): "));
        }
    }
    message.push_str(&v.diagnostic);
    if !v.suggested_fix.is_empty() {
        message.push_str("\n\nfix: ");
        message.push_str(&v.suggested_fix);
    }

    Value::Obj(vec![
        ("range".into(), range(line0, 0, line0, end_char)),
        ("severity".into(), Value::int(1)), // DiagnosticSeverity.Error
        ("code".into(), Value::str(v.k_class)),
        ("source".into(), Value::str("ufa-baremetal")),
        ("message".into(), Value::str(message)),
    ])
}

fn parse_error_diagnostic(e: &ParseCompletenessError) -> Value {
    // Surfaced so the editor shows *something* explaining why no K-check
    // diagnostics appear, instead of a silently-empty Problems panel --
    // audit_text refuses to run any check at all on a partial parse (see
    // its own doc comment / the Council audit it cites).
    Value::Obj(vec![
        ("range".into(), range(0, 0, 0, 1)),
        ("severity".into(), Value::int(1)),
        ("source".into(), Value::str("ufa-baremetal")),
        ("message".into(), Value::str(format!("parse-completeness preflight failed; no K-checks were run: {e}"))),
    ])
}

fn range(start_line: u32, start_char: u32, end_line: u32, end_char: u32) -> Value {
    Value::Obj(vec![
        ("start".into(), position(start_line, start_char)),
        ("end".into(), position(end_line, end_char)),
    ])
}

fn position(line: u32, character: u32) -> Value {
    Value::Obj(vec![("line".into(), Value::int(line as i64)), ("character".into(), Value::int(character as i64))])
}

/// LSP `Position.character` is defined in UTF-16 code units (the base
/// protocol's historical default encoding), not bytes or `char`s. For
/// the ASCII-heavy `.s`/comment text this server actually sees these are
/// all the same number, but computing it properly costs nothing and
/// avoids a subtly-wrong highlight range the one time a comment has a
/// non-ASCII character in it.
fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_value::parse as parse_json;

    fn balanced_fn(name: &str) -> String {
        format!(
            "\
.text
.global {name}
.type {name}, %function
{name}:
    STP X29, X30, [SP, #-16]!
    SUB SP, SP, #16
    MOV X29, SP
    ADD SP, SP, #16
    LDP X29, X30, [SP], #16
    RET
.size {name}, . - {name}
"
        )
    }

    fn imbalanced_fn(name: &str) -> String {
        format!(
            "\
.text
.global {name}
.type {name}, %function
{name}:
    STP X29, X30, [SP, #-16]!
    SUB SP, SP, #32
    MOV X29, SP
    ADD SP, SP, #16
    LDP X29, X30, [SP], #16
    RET
.size {name}, . - {name}
"
        )
    }

    fn notif(method: &str, params: Value) -> Value {
        Value::Obj(vec![("jsonrpc".into(), Value::str("2.0")), ("method".into(), Value::str(method)), ("params".into(), params)])
    }

    fn req(id: i64, method: &str, params: Value) -> Value {
        Value::Obj(vec![
            ("jsonrpc".into(), Value::str("2.0")),
            ("id".into(), Value::int(id)),
            ("method".into(), Value::str(method)),
            ("params".into(), params),
        ])
    }

    fn text_document_params(uri: &str, text: &str, version: i64) -> Value {
        Value::Obj(vec![(
            "textDocument".into(),
            Value::Obj(vec![
                ("uri".into(), Value::str(uri)),
                ("languageId".into(), Value::str("asm")),
                ("version".into(), Value::int(version)),
                ("text".into(), Value::str(text)),
            ]),
        )])
    }

    /// Drains every framed message currently in `buf` into parsed JSON
    /// values, in order, and empties `buf` -- lets each test read "what
    /// got sent" without hand-parsing the wire format.
    fn drain_messages(buf: &mut Vec<u8>) -> Vec<Value> {
        let mut reader = std::io::BufReader::new(std::io::Cursor::new(std::mem::take(buf)));
        let mut out = Vec::new();
        while let Some(body) = rpc::read_message(&mut reader).unwrap() {
            out.push(parse_json(&body).unwrap());
        }
        out
    }

    fn initialized_server() -> (Server, Vec<u8>) {
        let mut server = Server::new();
        let mut out = Vec::new();
        assert!(!server.handle(req(1, "initialize", Value::Obj(vec![])), &mut out));
        out.clear();
        assert!(!server.handle(notif("initialized", Value::Obj(vec![])), &mut out));
        out.clear();
        (server, out)
    }

    #[test]
    fn initialize_advertises_full_sync_and_hover() {
        let mut server = Server::new();
        let mut out = Vec::new();
        server.handle(req(1, "initialize", Value::Obj(vec![])), &mut out);
        let msgs = drain_messages(&mut out);
        assert_eq!(msgs.len(), 1);
        let caps = msgs[0].get("result").unwrap().get("capabilities").unwrap();
        assert_eq!(caps.get("textDocumentSync").unwrap().get("change").unwrap().as_i64(), Some(1));
        assert_eq!(caps.get("hoverProvider").unwrap(), &Value::Bool(true));
    }

    #[test]
    fn requests_before_initialize_get_an_error_not_a_hang() {
        let mut server = Server::new();
        let mut out = Vec::new();
        server.handle(req(1, "textDocument/hover", Value::Obj(vec![])), &mut out);
        let msgs = drain_messages(&mut out);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].get("error").is_some());
    }

    #[test]
    fn did_open_with_a_frame_imbalance_publishes_a_k01_fail_diagnostic() {
        let (mut server, mut out) = initialized_server();
        let src = imbalanced_fn("bad_fn");
        server.handle(notif("textDocument/didOpen", text_document_params("file:///tmp/bad.s", &src, 1)), &mut out);

        let msgs = drain_messages(&mut out);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get("method").unwrap().as_str(), Some("textDocument/publishDiagnostics"));
        let params = msgs[0].get("params").unwrap();
        assert_eq!(params.get("uri").unwrap().as_str(), Some("file:///tmp/bad.s"));
        let diags = params.get("diagnostics").unwrap().as_array().unwrap();
        assert!(!diags.is_empty());
        assert!(diags.iter().any(|d| d.get("code").unwrap().as_str() == Some("K01")));
        let d = &diags[0];
        assert_eq!(d.get("severity").unwrap().as_i64(), Some(1));
        // start_line == 3 (0-based) because the function label + SUB SP
        // line is physical line 4 in `imbalanced_fn`'s literal above.
        assert_eq!(d.get("range").unwrap().get("start").unwrap().get("line").unwrap().as_i64(), Some(3));
    }

    #[test]
    fn did_open_with_a_clean_file_publishes_no_diagnostics() {
        let (mut server, mut out) = initialized_server();
        let src = balanced_fn("good_fn");
        server.handle(notif("textDocument/didOpen", text_document_params("file:///tmp/good.s", &src, 1)), &mut out);
        let msgs = drain_messages(&mut out);
        let diags = msgs[0].get("params").unwrap().get("diagnostics").unwrap().as_array().unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn non_dot_s_uris_are_never_checked() {
        let (mut server, mut out) = initialized_server();
        server.handle(notif("textDocument/didOpen", text_document_params("file:///tmp/notes.txt", "anything", 1)), &mut out);
        assert!(drain_messages(&mut out).is_empty(), "no publishDiagnostics should be sent for a non-.s file");
    }

    #[test]
    fn did_change_full_sync_updates_diagnostics_from_bad_to_clean() {
        let (mut server, mut out) = initialized_server();
        let bad = imbalanced_fn("f");
        server.handle(notif("textDocument/didOpen", text_document_params("file:///tmp/f.s", &bad, 1)), &mut out);
        assert!(!drain_messages(&mut out)[0].get("params").unwrap().get("diagnostics").unwrap().as_array().unwrap().is_empty());

        let good = balanced_fn("f");
        let change_params = Value::Obj(vec![
            (
                "textDocument".into(),
                Value::Obj(vec![("uri".into(), Value::str("file:///tmp/f.s")), ("version".into(), Value::int(2))]),
            ),
            ("contentChanges".into(), Value::Arr(vec![Value::Obj(vec![("text".into(), Value::str(good))])])),
        ]);
        server.handle(notif("textDocument/didChange", change_params), &mut out);
        let msgs = drain_messages(&mut out);
        let diags = msgs[0].get("params").unwrap().get("diagnostics").unwrap().as_array().unwrap();
        assert!(diags.is_empty(), "diagnostics should clear once the buffer is fixed");
    }

    #[test]
    fn did_change_with_a_ranged_edit_is_ignored_not_misapplied() {
        let (mut server, mut out) = initialized_server();
        let bad = imbalanced_fn("f");
        server.handle(notif("textDocument/didOpen", text_document_params("file:///tmp/f.s", &bad, 1)), &mut out);
        out.clear();

        let ranged_change = Value::Obj(vec![
            (
                "textDocument".into(),
                Value::Obj(vec![("uri".into(), Value::str("file:///tmp/f.s")), ("version".into(), Value::int(2))]),
            ),
            (
                "contentChanges".into(),
                Value::Arr(vec![Value::Obj(vec![
                    (
                        "range".into(),
                        Value::Obj(vec![
                            ("start".into(), Value::Obj(vec![("line".into(), Value::int(0)), ("character".into(), Value::int(0))])),
                            ("end".into(), Value::Obj(vec![("line".into(), Value::int(0)), ("character".into(), Value::int(1))])),
                        ]),
                    ),
                    ("text".into(), Value::str("X")),
                ])]),
            ),
        ]);
        server.handle(notif("textDocument/didChange", ranged_change), &mut out);
        // No publish at all -- we bail out before touching stored state
        // or re-checking, rather than guessing at a patch we can't apply.
        assert!(drain_messages(&mut out).is_empty());
    }

    #[test]
    fn did_close_clears_previously_published_diagnostics() {
        let (mut server, mut out) = initialized_server();
        let bad = imbalanced_fn("f");
        server.handle(notif("textDocument/didOpen", text_document_params("file:///tmp/f.s", &bad, 1)), &mut out);
        out.clear();

        let close_params = Value::Obj(vec![("textDocument".into(), Value::Obj(vec![("uri".into(), Value::str("file:///tmp/f.s"))]))]);
        server.handle(notif("textDocument/didClose", close_params), &mut out);
        let msgs = drain_messages(&mut out);
        assert_eq!(msgs.len(), 1);
        let diags = msgs[0].get("params").unwrap().get("diagnostics").unwrap().as_array().unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn hover_on_a_fail_line_returns_markdown_and_elsewhere_returns_null() {
        let (mut server, mut out) = initialized_server();
        let bad = imbalanced_fn("f");
        server.handle(notif("textDocument/didOpen", text_document_params("file:///tmp/f.s", &bad, 1)), &mut out);
        out.clear();

        let hover_params = |line: i64| {
            Value::Obj(vec![
                ("textDocument".into(), Value::Obj(vec![("uri".into(), Value::str("file:///tmp/f.s"))])),
                ("position".into(), Value::Obj(vec![("line".into(), Value::int(line)), ("character".into(), Value::int(0))])),
            ])
        };

        server.handle(req(9, "textDocument/hover", hover_params(3)), &mut out);
        let msgs = drain_messages(&mut out);
        let contents = msgs[0].get("result").unwrap().get("contents").unwrap();
        assert!(contents.get("value").unwrap().as_str().unwrap().contains("K01"));

        server.handle(req(10, "textDocument/hover", hover_params(0)), &mut out);
        let msgs = drain_messages(&mut out);
        assert_eq!(msgs[0].get("result").unwrap(), &Value::Null);
    }

    #[test]
    fn parse_completeness_failure_surfaces_as_a_diagnostic_instead_of_silence() {
        let (mut server, mut out) = initialized_server();
        let src = ".text\n.type ghost, %function\n// ghost: never appears as a label\n";
        server.handle(notif("textDocument/didOpen", text_document_params("file:///tmp/ghost.s", src, 1)), &mut out);
        let msgs = drain_messages(&mut out);
        let diags = msgs[0].get("params").unwrap().get("diagnostics").unwrap().as_array().unwrap();
        assert_eq!(diags.len(), 1);
        assert!(diags[0].get("message").unwrap().as_str().unwrap().contains("parse-completeness"));
    }

    #[test]
    fn unknown_request_gets_method_not_found_unknown_notification_is_silent() {
        let (mut server, mut out) = initialized_server();
        server.handle(req(5, "textDocument/definition", Value::Obj(vec![])), &mut out);
        let msgs = drain_messages(&mut out);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get("error").unwrap().get("code").unwrap().as_i64(), Some(-32601));

        server.handle(notif("textDocument/definition", Value::Obj(vec![])), &mut out);
        assert!(drain_messages(&mut out).is_empty());
    }

    #[test]
    fn shutdown_then_exit_signals_the_read_loop_to_stop() {
        let (mut server, mut out) = initialized_server();
        assert!(!server.handle(req(1, "shutdown", Value::Null), &mut out));
        let msgs = drain_messages(&mut out);
        assert_eq!(msgs[0].get("result").unwrap(), &Value::Null);
        assert!(server.handle(notif("exit", Value::Null), &mut out), "exit must signal the loop to stop");
    }

    #[test]
    fn uri_to_display_name_strips_scheme_dir_and_percent_encoding() {
        assert_eq!(uri_to_display_name("file:///Users/dev/My%20Project/foo.s"), "foo.s");
        assert_eq!(uri_to_display_name("file:///a/b/c.s"), "c.s");
    }

    #[test]
    fn should_check_only_matches_dot_s_paths() {
        assert!(should_check("file:///a/b.s"));
        assert!(should_check("file:///a/b.S"), "extension match is case-insensitive");
        assert!(!should_check("file:///a/b.txt"));
        assert!(!should_check("file:///a/b.s.bak"));
    }
}
