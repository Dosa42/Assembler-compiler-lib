//! WASM entry point for the browser playground (roadmap item #6): "paste
//! `.s`, see verdicts, no server, no install."
//!
//! The `extern "C"` interface uses pointers, lengths, and a check mask, without
//! `wasm-bindgen`: this keeps the same "stdlib only, hand-roll the small
//! fixed surface you actually need" discipline
//! [`ufa_baremetal_core`](../ufa_baremetal_core/index.html) already
//! applies to JSON/SHA-256/datetime (see that crate's `Cargo.toml`),
//! extended here to its WASM consumer. It also means there's no
//! `wasm-bindgen-cli` version to keep in lockstep with the `wasm-bindgen`
//! crate version, and no separate JS build step — the playground loads
//! the compiled module directly.
//!
//! The contract with `ufa-baremetal-playground.html`, end to end:
//!
//! 1. JS calls [`alloc`] twice: once for the pasted `.s` text, once for
//!    a filename (UTF-8, e.g. `"pasted.s"` or `"pasted_boot.s"` — see
//!    below), and writes each via a `Uint8Array` view over
//!    `memory.buffer`. JS has no other way to get a valid wasm
//!    linear-memory address to write at.
//! 2. JS calls [`check`] or [`check_selected`] with both pointer/length pairs.
//!    The latter also takes a bit mask. The result comes back
//!    as one packed `u64`: the high 32 bits are a pointer to a UTF-8
//!    JSON document, the low 32 bits are its byte length (see
//!    [`check`]'s doc for the packing and the JSON shape).
//! 3. JS reads that many bytes back out of memory, `JSON.parse()`s them,
//!    renders the result, then calls [`dealloc`] on *all three* buffers
//!    (the text and filename it wrote in step 1, and the output
//!    `check` returned) once it's done with them, since nothing on the
//!    Rust side frees any of them automatically.
//!
//! The filename isn't cosmetic: [`k_invariants::k05_cache_interrupt_hygiene`]
//! (K05) only applies to files whose name ends in `_boot.s` or `_init.s`
//! — that's checked against this exact string, the same way it's
//! checked against a real path's basename in the CLI. A playground that
//! silently hardcoded one constant name here would make K05 permanently
//! unreachable from the browser; the playground exposes a filename
//! field precisely so it doesn't.
//!
//! Packing a pointer and a length into one `u64` return value (rather
//! than returning a struct or a pair) is the standard workaround for
//! wasm32-unknown-unknown's default C ABI only supporting a single
//! i32/i64 return per call. Both halves comfortably fit 32 bits — a
//! multi-GB `.s` file is not a shape this tool is meant to handle, on
//! the browser path or any other.

use std::mem;
use std::slice;

use ufa_baremetal_core::codegen_audit::audit_text_selected;
use ufa_baremetal_core::json::Json;
use ufa_baremetal_core::verdict::Verdict;

/// Allocate `len` bytes inside this module's linear memory and return a
/// pointer to them, for the JS side to write the pasted `.s` text into
/// before calling [`check`]. JS has no other way to get a valid wasm
/// linear-memory address to write at — it can't just pick one.
///
/// The returned pointer is owned by the caller until it's passed to
/// [`dealloc`] with this same `len`.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf: Vec<u8> = Vec::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    // Hand the buffer's memory to the caller: `mem::forget` skips
    // running `Vec`'s destructor (which would otherwise free this on
    // the spot), without touching the bytes themselves. Ownership now
    // lives on the JS side until it round-trips back through `dealloc`.
    mem::forget(buf);
    ptr
}

/// Free a buffer previously handed out by [`alloc`] (including the
/// output buffer [`check`] returns a pointer into) once the JS side is
/// done reading it. `len` must be the exact length that buffer was
/// allocated with — reconstructing a `Vec` from a raw pointer is only
/// sound if the capacity matches what it was actually allocated with.
#[no_mangle]
pub extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    unsafe {
        drop(Vec::from_raw_parts(ptr, len, len));
    }
}

/// Run every check in `K_REGISTRY` over the UTF-8 text previously
/// written at `text_ptr..text_ptr+text_len` (via [`alloc`]), using the
/// UTF-8 name at `name_ptr..name_ptr+name_len` (also via [`alloc`]) both
/// as the filename [`Verdict::file`] is tagged with and as the name
/// K05's `_boot.s`/`_init.s` applicability check runs against — see the
/// module doc for why that name has to be a real parameter here rather
/// than a hardcoded constant. Returns the result as a pointer+length
/// pair packed into one `u64`: `(ptr as u64) << 32 | len`.
///
/// The bytes at that returned pointer are a UTF-8 JSON document, always
/// one of two shapes:
///
/// - On a clean parse: a JSON array of verdict objects, each shaped
///   like one entry of the CLI's own `--json` `"verdicts"` array
///   (`status`, `k_class`, `function`, `line_no`, `diagnostic`,
///   `suggested_fix` — `file` is omitted here since the caller already
///   supplied that same name and knows it).
/// - On a parse-completeness failure (see
///   [`ufa_baremetal_core::parse_s::assert_parse_complete`]): a JSON
///   object `{"error": "..."}` instead, so the JS side has exactly one
///   shape to branch on (array vs. object) rather than a second calling
///   convention for failure.
///
/// Both inputs are decoded with [`String::from_utf8_lossy`] rather than
/// rejected outright on invalid UTF-8: text from a browser textarea or
/// input field is already JS-string (UTF-16) re-encoded to UTF-8 by
/// `TextEncoder`, so it can't actually contain invalid UTF-8 in
/// practice — lossy decoding here is a defensive fallback, not a
/// behavior this path is expected to exercise.
#[no_mangle]
pub extern "C" fn check(text_ptr: *mut u8, text_len: usize, name_ptr: *mut u8, name_len: usize) -> u64 {
    check_selected(text_ptr, text_len, name_ptr, name_len, u32::MAX)
}

/// The same input and result buffers as [`check`], with bits 0 through 5
/// selecting K01 through K06 respectively.
#[no_mangle]
pub extern "C" fn check_selected(
    text_ptr: *mut u8,
    text_len: usize,
    name_ptr: *mut u8,
    name_len: usize,
    mask: u32,
) -> u64 {
    let text_bytes = unsafe { slice::from_raw_parts(text_ptr, text_len) };
    let text = String::from_utf8_lossy(text_bytes);
    let name_bytes = unsafe { slice::from_raw_parts(name_ptr, name_len) };
    let name = String::from_utf8_lossy(name_bytes);

    let json = match audit_text_selected(&text, &name, mask) {
        Ok(verdicts) => Json::Array(verdicts.iter().map(verdict_to_json).collect()),
        Err(e) => Json::Object(vec![("error".into(), Json::str(e.to_string()))]),
    };

    let out = json.to_pretty_string().into_bytes();
    let out_len = out.len();
    let out_ptr = alloc(out_len);
    unsafe {
        std::ptr::copy_nonoverlapping(out.as_ptr(), out_ptr, out_len);
    }
    ((out_ptr as u64) << 32) | (out_len as u64)
}

/// One verdict, as a JSON object. Field set and order intentionally
/// mirrors `ufa-baremetal --json`'s per-verdict shape (see
/// `bin/ufa-baremetal/src/main.rs::emit_json`) minus `file`, so the two
/// front ends stay easy to cross-reference even though nothing here
/// shares code with that CLI-only function.
fn verdict_to_json(v: &Verdict) -> Json {
    Json::Object(vec![
        ("status".into(), Json::str(v.status.as_str())),
        ("k_class".into(), Json::str(v.k_class)),
        ("function".into(), Json::opt_str(v.function.clone())),
        ("line_no".into(), Json::opt_u32(v.line_no)),
        ("diagnostic".into(), Json::str(v.diagnostic.clone())),
        ("suggested_fix".into(), Json::str(v.suggested_fix.clone())),
    ])
}
