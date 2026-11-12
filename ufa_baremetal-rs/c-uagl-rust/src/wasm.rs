//! In-memory WebAssembly bindings for the three existing file operations.
//!
//! Write the UTF-8 JSON request into a buffer returned by `alloc`. Each
//! operation returns a `u64` with the output pointer in the high 32 bits and
//! the byte length in the low 32 bits. Read the UTF-8 JSON result and free
//! both buffers with `dealloc`.

use crate::{fidelity, linux_converter, uagl2c, validator};
use serde_json::{json, Value};
use std::{ptr, slice};

fn into_buffer(bytes: Box<[u8]>) -> *mut u8 {
    Box::into_raw(bytes).cast::<u8>()
}

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    into_buffer(vec![0; len].into_boxed_slice())
}

/// # Safety
/// `pointer` and `len` must identify an allocation from this module. After
/// this call the caller must stop accessing the buffer.
#[no_mangle]
pub unsafe extern "C" fn dealloc(pointer: *mut u8, len: usize) {
    unsafe { drop(Box::from_raw(ptr::slice_from_raw_parts_mut(pointer, len))) };
}

fn text_field<'a>(request: &'a Value, key: &str) -> Result<&'a str, String> {
    request[key]
        .as_str()
        .ok_or_else(|| format!("Expected a string in {key}"))
}

fn respond(request: &[u8], operation: fn(&Value) -> Result<Value, String>) -> u64 {
    let result = serde_json::from_slice(request)
        .map_err(|error| error.to_string())
        .and_then(|value| operation(&value));
    let result = match result {
        Ok(value) => value,
        Err(error) => json!({ "error": error }),
    };
    let bytes = result.to_string().into_bytes().into_boxed_slice();
    let len = bytes.len();
    let pointer = into_buffer(bytes) as usize;
    ((pointer as u64) << 32) | len as u64
}

/// Request fields: `source`, `source_path`, `output_path`, and optionally
/// `validate` (defaults to true). Returns `uagl`, `gaps_json`, and `validation`.
///
/// # Safety
/// `pointer` and `len` must identify a readable request buffer in Wasm memory.
#[no_mangle]
pub unsafe extern "C" fn convert_c(pointer: *const u8, len: usize) -> u64 {
    respond(unsafe { slice::from_raw_parts(pointer, len) }, |request| {
        let source = text_field(request, "source")?;
        let source_path = text_field(request, "source_path")?;
        let output_path = text_field(request, "output_path")?;
        let validate = match request.get("validate") {
            Some(value) => value
                .as_bool()
                .ok_or_else(|| "Expected a boolean in validate".to_owned())?,
            None => true,
        };
        let converted = linux_converter::convert_source(source, source_path)
            .map_err(|error| error.to_string())?;
        let validation = validate.then(|| {
            validator::format_result(
                &validator::UaglValidator::new().validate_source(&converted.uagl, output_path),
            )
        });
        let gaps = linux_converter::gap_manifest(source_path, output_path, &converted.gaps);
        Ok(json!({ "uagl": converted.uagl, "gaps_json": gaps, "validation": validation }))
    })
}

/// Request field: `uagl`. Returns `c`.
///
/// # Safety
/// `pointer` and `len` must identify a readable request buffer in Wasm memory.
#[no_mangle]
pub unsafe extern "C" fn uagl_to_c(pointer: *const u8, len: usize) -> u64 {
    respond(unsafe { slice::from_raw_parts(pointer, len) }, |request| {
        Ok(json!({ "c": uagl2c::convert(text_field(request, "uagl")?)? }))
    })
}

/// Request fields: `c_path`, `c`, `uagl_path`, and `uagl`. Returns `report`.
///
/// # Safety
/// `pointer` and `len` must identify a readable request buffer in Wasm memory.
#[no_mangle]
pub unsafe extern "C" fn fidelity_report(pointer: *const u8, len: usize) -> u64 {
    respond(unsafe { slice::from_raw_parts(pointer, len) }, |request| {
        let report = fidelity::report(
            text_field(request, "c_path")?,
            text_field(request, "c")?,
            text_field(request, "uagl_path")?,
            text_field(request, "uagl")?,
        )?;
        Ok(json!({ "report": report }))
    })
}
