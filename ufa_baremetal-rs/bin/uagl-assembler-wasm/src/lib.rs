//! WebAssembly interface to the UAGL assembler library.

use std::{ptr, slice};

use serde_json::json;
use uagl_aarch64_assembler::{assemble_uagl_bytes, tables::reset_all_tables, validate_uagl_bytes};

fn into_buffer(bytes: Box<[u8]>) -> *mut u8 {
    Box::into_raw(bytes).cast::<u8>()
}

/// Allocate a browser input buffer in the module's linear memory.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    into_buffer(vec![0; len].into_boxed_slice())
}

/// Free one input or output buffer.
///
/// # Safety
/// `pointer` and `len` must identify a live buffer returned by `alloc`,
/// `assemble`, or `validate`. The caller must stop accessing it after this call.
#[no_mangle]
pub unsafe extern "C" fn dealloc(pointer: *mut u8, len: usize) {
    let buffer = ptr::slice_from_raw_parts_mut(pointer, len);
    unsafe { drop(Box::from_raw(buffer)) };
}

/// Translate UAGL text or a `.uaglbr` container into AArch64 assembly text.
/// The high 32 result bits are the JSON buffer pointer and the low 32 are
/// its length. The browser owns the result buffer and must free it.
///
/// # Safety
/// Both pointer/length pairs must identify initialized, readable byte slices
/// in the module's memory. The caller retains ownership of the input buffers.
#[no_mangle]
pub unsafe extern "C" fn assemble(
    source_pointer: *const u8,
    source_len: usize,
    name_pointer: *const u8,
    name_len: usize,
) -> u64 {
    let source = unsafe { slice::from_raw_parts(source_pointer, source_len) };
    let name = unsafe { slice::from_raw_parts(name_pointer, name_len) };
    let name = String::from_utf8_lossy(name);

    // Each playground action compiles one independent file, as the native CLI does.
    reset_all_tables();
    let result = match assemble_uagl_bytes(source, &name) {
        Ok(assembly) => json!({ "assembly": assembly }),
        Err(error) => json!({ "error": error }),
    };

    pack_json(result)
}

/// Check UAGL input and return line diagnostics using the `assemble` result ABI.
///
/// # Safety
/// Both pointer/length pairs must identify initialized, readable byte slices
/// in the module's memory. The caller retains ownership of the input buffers.
#[no_mangle]
pub unsafe extern "C" fn validate(
    source_pointer: *const u8,
    source_len: usize,
    name_pointer: *const u8,
    name_len: usize,
) -> u64 {
    let source = unsafe { slice::from_raw_parts(source_pointer, source_len) };
    let name = unsafe { slice::from_raw_parts(name_pointer, name_len) };
    let name = String::from_utf8_lossy(name);

    reset_all_tables();
    let result = match validate_uagl_bytes(source, &name) {
        Ok(diagnostics) => {
            let entries = diagnostics.into_iter().map(|diagnostic| {
                json!({ "line": diagnostic.line, "message": diagnostic.message })
            }).collect::<Vec<_>>();
            json!({ "diagnostics": entries })
        }
        Err(error) => json!({ "error": error }),
    };
    pack_json(result)
}

fn pack_json(result: serde_json::Value) -> u64 {
    let bytes = result.to_string().into_bytes().into_boxed_slice();
    let len = bytes.len();
    let pointer = into_buffer(bytes) as usize;
    ((pointer as u64) << 32) | (len as u64)
}
