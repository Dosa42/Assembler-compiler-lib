//! `ufa-baremetal-lsp` -- a Language Server wrapping the K01-K06 check
//! pipeline (ROADMAP.md item #4), so violations show up as native
//! diagnostics in any LSP-capable editor as the `.s` buffer is edited,
//! not just when the CLI is run by hand or in CI.
//!
//! Speaks LSP over stdio: reads `Content-Length`-framed JSON-RPC
//! requests/notifications on stdin, writes framed responses/notifications
//! to stdout. Every editor integration in `editors/` at the workspace
//! root launches this binary with no arguments and talks to it this way
//! -- there is nothing editor-specific in this crate itself, which is
//! the entire point of building this as an LSP rather than a bespoke
//! VS-Code-only extension (see ROADMAP.md item #4's own reasoning).
//!
//! This binary owns all the I/O (stdin/stdout, the in-memory document
//! store) and none of the check logic -- every diagnostic comes from
//! `ufa_baremetal_core::codegen_audit::audit_text`, the exact same
//! function-per-check pipeline the CLI (`bin/ufa-baremetal`) runs. A K07
//! added to `k_invariants.rs`'s `K_REGISTRY` needs zero changes here to
//! start showing up as a live diagnostic.
//!
//! stdout is reserved entirely for framed JSON-RPC messages, per the LSP
//! base protocol -- this binary never `println!`s. Anything worth
//! surfacing outside the protocol (malformed input from the client, I/O
//! errors) goes to stderr instead, which editors typically capture into
//! an output/log channel rather than mixing into the protocol stream.

mod json_value;
mod protocol;
mod rpc;

use std::io::{self, BufReader};

fn main() {
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut writer = stdout.lock();

    let mut server = protocol::Server::new();

    loop {
        match rpc::read_message(&mut reader) {
            Ok(Some(body)) => match json_value::parse(&body) {
                Ok(msg) => {
                    if server.handle(msg, &mut writer) {
                        break; // `exit` notification received
                    }
                }
                Err(e) => {
                    eprintln!("ufa-baremetal-lsp: dropping malformed JSON-RPC message: {e}");
                }
            },
            Ok(None) => break, // stdin closed -- the client is gone
            Err(e) => {
                eprintln!("ufa-baremetal-lsp: I/O error reading from stdin, stopping: {e}");
                break;
            }
        }
    }
}
