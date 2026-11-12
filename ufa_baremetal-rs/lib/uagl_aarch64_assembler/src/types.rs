//! UAGL type sizing, alignment, and struct layout — a faithful port of
//! `type_size`, `_type_size_align`, `_compute_struct_layout`, `_field_offset`,
//! `_field_type`, `_elem_log2`, `_struct_field_elem_stride`, and
//! `_global_array_elem_stride`.

use crate::tables::{GLOBAL_TABLE, STRUCT_LAYOUTS, STRUCT_TABLE};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

static ARRAY_TYPE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([A-Za-z_]\w*)\s*\[(\d+)\]$").unwrap());

fn primitive_size(ftype: &str) -> Option<i64> {
    Some(match ftype {
        "U8" | "UINT8" | "I8" | "INT8" | "BOOL" | "CHAR" => 1,
        "U16" | "UINT16" | "I16" | "INT16" => 2,
        "U32" | "UINT32" | "I32" | "INT32" | "S32" | "FLOAT" => 4,
        "U64" | "UINT64" | "I64" | "INT64" | "S64" => 8,
        "F64" | "DOUBLE" => 8,
        "GVector" => 32,
        _ => return None,
    })
}

/// Return (size_bytes, alignment_bytes) for a UAGL type, recursing into
/// nested struct fields when needed.
///
/// Faithfully preserves an upstream quirk: a struct *field* declared with a
/// pointer type (`@Foo`) has its leading `@` stripped here before the struct
/// lookup, so it is sized as an INLINE `Foo` rather than as an 8-byte
/// pointer. `type_size()` below guards local/parameter allocation against
/// this (it short-circuits to (8, 8) whenever the untouched type string
/// contains '@'), so in practice the effect is scoped to pointer-typed
/// struct *fields* specifically. See the port notes for detail — this is
/// preserved as-is rather than "fixed", since fixing it would be a
/// behavior change beyond a straight language port.
pub fn type_size_align(ftype: &str) -> (i64, i64) {
    let ftype = ftype.trim().trim_start_matches('@');
    if ftype == "PTR" {
        return (8, 8);
    }
    if let Some(caps) = ARRAY_TYPE_RE.captures(ftype) {
        let elem_t = &caps[1];
        let n: i64 = caps[2].parse().unwrap_or(0);
        let (elem_sz, elem_align) = type_size_align(elem_t);
        return (n * elem_sz, elem_align);
    }
    if let Some(sz) = primitive_size(ftype) {
        return (sz, if sz <= 8 { sz } else { 8 });
    }
    if STRUCT_TABLE.lock().unwrap().contains_key(ftype) {
        let (_, total) = compute_struct_layout(ftype);
        return (total, 8);
    }
    (8, 8)
}

/// Byte size of a UAGL type. Recurses into user structs via the layout
/// cache; empty/unsized arrays default to 8 (PTR). Mirrors `type_size`.
pub fn type_size(utype: &str) -> i64 {
    let sz = if utype.contains('@') {
        8
    } else {
        type_size_align(utype).0
    };
    if utype.trim() == "VOID" {
        return 0;
    }
    sz
}

/// Resolve struct field offsets + total size, with natural alignment per
/// field. Cached so repeated lookups don't re-walk the field list. Mirrors
/// `_compute_struct_layout`, including its self-referential-field
/// cache-preseeding trick: the partial layout is written back to the shared
/// cache after each field so a field whose type resolves back to this same
/// struct (a self-pointer) sees the fields declared before it instead of
/// recursing forever.
pub fn compute_struct_layout(struct_name: &str) -> (HashMap<String, i64>, i64) {
    if let Some(existing) = STRUCT_LAYOUTS.lock().unwrap().get(struct_name) {
        return existing.clone();
    }

    let fields: Vec<(String, String)> = STRUCT_TABLE
        .lock()
        .unwrap()
        .get(struct_name)
        .cloned()
        .unwrap_or_default();

    let mut layout: HashMap<String, i64> = HashMap::new();
    let mut offset: i64 = 0;
    let mut max_align: i64 = 1;

    STRUCT_LAYOUTS
        .lock()
        .unwrap()
        .insert(struct_name.to_string(), (layout.clone(), 0));

    for (fname, ftype) in &fields {
        let (sz, align) = type_size_align(ftype);
        if align > 1 {
            offset = (offset + align - 1) & !(align - 1);
        }
        layout.insert(fname.clone(), offset);
        offset += sz;
        if align > max_align {
            max_align = align;
        }
        // Refresh the shared partial view for any reentrant lookup
        // triggered by a later field's `type_size_align` call.
        STRUCT_LAYOUTS
            .lock()
            .unwrap()
            .insert(struct_name.to_string(), (layout.clone(), 0));
    }
    if max_align > 1 {
        offset = (offset + max_align - 1) & !(max_align - 1);
    }
    let result = (layout, offset);
    STRUCT_LAYOUTS
        .lock()
        .unwrap()
        .insert(struct_name.to_string(), result.clone());
    result
}

/// Byte offset of a field within a struct (after layout caching).
pub fn field_offset(struct_name: &str, field_name: &str) -> i64 {
    let sname = struct_name.trim_start_matches('@');
    let (layout, _) = compute_struct_layout(sname);
    *layout.get(field_name).unwrap_or(&0)
}

/// Declared type of a struct field, or "" if unknown.
pub fn field_type(struct_name: &str, field_name: &str) -> String {
    let sname = struct_name.trim_start_matches('@');
    STRUCT_TABLE
        .lock()
        .unwrap()
        .get(sname)
        .and_then(|fields| fields.iter().find(|(n, _)| n == field_name))
        .map(|(_, t)| t.clone())
        .unwrap_or_default()
}

/// log2(byte stride) for an element type. Falls through to struct sizes via
/// `STRUCT_TABLE` when `elem` names a user struct: returns log2 of the
/// struct size when it's a power of two. For non-power-of-two struct
/// sizes, returns 3 (8-byte).
pub fn elem_log2(elem: &str) -> i64 {
    match elem {
        "UINT8" | "U8" | "INT8" | "S8" | "BOOL" | "CHAR" => return 0,
        "UINT16" | "U16" | "INT16" | "S16" => return 1,
        "UINT32" | "U32" | "INT32" | "S32" | "FLOAT" => return 2,
        _ => {}
    }
    if STRUCT_TABLE.lock().unwrap().contains_key(elem) {
        let (_, total) = compute_struct_layout(elem);
        if total > 0 && (total & (total - 1)) == 0 {
            return total.trailing_zeros() as i64;
        }
    }
    3
}

/// Look up element type & log2 stride for a struct array field. Falls back
/// to `default_elem` when the lookup fails (unknown struct or field).
pub fn struct_field_elem_stride(
    struct_name: &str,
    field_name: &str,
    default_elem: &str,
) -> (String, i64) {
    let sname = struct_name.trim_start_matches('@');
    let ftype = STRUCT_TABLE
        .lock()
        .unwrap()
        .get(sname)
        .and_then(|fields| fields.iter().find(|(n, _)| n == field_name))
        .map(|(_, t)| t.clone());
    let elem = match &ftype {
        Some(t) if t.contains('[') => t[..t.find('[').unwrap()].to_string(),
        _ => default_elem.to_string(),
    };
    let log2 = elem_log2(&elem);
    (elem, log2)
}

/// Look up element type & log2 stride for a global array (or scalar).
/// Falls back to `default_elem` when the symbol isn't registered.
pub fn global_array_elem_stride(name: &str, default_elem: &str) -> (String, i64) {
    let elem = GLOBAL_TABLE
        .lock()
        .unwrap()
        .get(name)
        .cloned()
        .unwrap_or_else(|| default_elem.to_string());
    let log2 = elem_log2(&elem);
    (elem, log2)
}

/// Insert-or-update-in-place, matching Python `dict[key] = value` semantics
/// (re-assigning an existing key updates its value without moving its
/// position; a new key is appended). Used when populating `STRUCT_TABLE`'s
/// per-struct field list, since `_STRUCT_TABLE[cur_struct][name] = type` in
/// the original is a genuine dict assignment.
pub fn dict_set(v: &mut Vec<(String, String)>, key: String, value: String) {
    if let Some(entry) = v.iter_mut().find(|(k, _)| *k == key) {
        entry.1 = value;
    } else {
        v.push((key, value));
    }
}
