//! Expression codegen — a faithful port of `_operand_aarch64_type`,
//! `_emit_struct_base_addr`, and the large `emit_load_value` dispatcher.
//!
//! `state` is modeled as `Option<&FuncState>` throughout (see the doc
//! comment on `FuncState::label_counter` for why a shared reference
//! suffices even though `emit_call` bumps a counter deep in this call
//! graph).

use crate::call::emit_call;
use crate::func_state::FuncState;
use crate::helpers::{
    aarch64_addsub_imm_lines, drop_first_last_char, has_top_level, split_binop, strip_outer_parens,
};
use crate::tables::GLOBAL_TABLE;
use crate::types::{elem_log2, field_offset, field_type, struct_field_elem_stride};
use once_cell::sync::Lazy;
use regex::Regex;

fn re(pat: &str) -> Regex {
    Regex::new(pat).unwrap_or_else(|e| panic!("bad regex {pat:?}: {e}"))
}

/// Best-effort type inference for an operand string. Recognises
/// params/locals via the function state, globals via `GLOBAL_TABLE`, and
/// array subscripts (returning the element type). Falls back to UINT64.
pub fn operand_aarch64_type(operand: &str, state: Option<&FuncState>) -> String {
    let mut operand = operand.trim().to_string();

    loop {
        if !(operand.starts_with('(') && operand.ends_with(')')) {
            break;
        }
        if !has_top_level(&drop_first_last_char(&operand), ' ') {
            break;
        }
        let stripped = strip_outer_parens(&operand);
        if stripped == operand {
            break;
        }
        operand = stripped;
    }

    if let Some(st) = state {
        for (pname, ptype) in &st.params {
            if pname == &operand {
                return ptype.clone();
            }
        }
        if st.offset_of(&operand).is_some() {
            return st.type_of(&operand);
        }
    }

    // Bug #44: struct field subscript: obj ─ △ field[idx] — element type
    // from the field's type.
    static FIELD_SUBSCRIPT_RE: Lazy<Regex> =
        Lazy::new(|| re(r"^(\w+)\s*─\s*△\s*(\w+)\s*\[[^\]]+\]$"));
    if let Some(caps) = FIELD_SUBSCRIPT_RE.captures(&operand) {
        let obj = &caps[1];
        let field = &caps[2];
        let mut obj_t: Option<String> = None;
        if let Some(st) = state {
            for (pname, ptype) in &st.params {
                if pname == obj {
                    obj_t = Some(ptype.trim_start_matches('@').to_string());
                    break;
                }
            }
            if obj_t.is_none() && st.offset_of(obj).is_some() {
                obj_t = Some(st.type_of(obj).trim_start_matches('@').to_string());
            }
        }
        if obj_t.is_none() {
            if let Some(t) = GLOBAL_TABLE.lock().unwrap().get(obj) {
                obj_t = Some(t.trim_start_matches('@').to_string());
            }
        }
        if let Some(ot) = &obj_t {
            if !ot.is_empty() {
                let (elem, _log2) = struct_field_elem_stride(ot, field, "INT32");
                return elem;
            }
        }
    }

    // Struct field (no subscript): obj ─ △ field
    static FIELD_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*─\s*△\s*(\w+)$"));
    if let Some(caps) = FIELD_RE.captures(&operand) {
        let obj = &caps[1];
        let field = &caps[2];
        let mut obj_t: Option<String> = None;
        if let Some(st) = state {
            for (pname, ptype) in &st.params {
                if pname == obj {
                    obj_t = Some(ptype.trim_start_matches('@').to_string());
                    break;
                }
            }
            if obj_t.is_none() && st.offset_of(obj).is_some() {
                obj_t = Some(st.type_of(obj).trim_start_matches('@').to_string());
            }
        }
        if let Some(ot) = &obj_t {
            if !ot.is_empty() {
                let ftype = field_type(ot, field);
                if !ftype.is_empty() {
                    return ftype.split('[').next().unwrap_or("").to_string();
                }
            }
        }
    }

    // Array subscript: return element type (decays @T or T[N] to T).
    static ARR_SUB_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\[([^\]]+)\]$"));
    if let Some(caps) = ARR_SUB_RE.captures(&operand) {
        let arr = &caps[1];
        let mut arr_t: Option<String> = None;
        if let Some(st) = state {
            if st.offset_of(arr).is_some() {
                arr_t = Some(st.type_of(arr));
            }
        }
        if arr_t.is_none() {
            arr_t = GLOBAL_TABLE.lock().unwrap().get(arr).cloned();
        }
        if let Some(at) = &arr_t {
            if !at.is_empty() {
                let at = at.trim();
                if let Some(stripped) = at.strip_prefix('@') {
                    return stripped.to_string();
                }
                if let Some(idx) = at.find('[') {
                    return at[..idx].to_string();
                }
                return at.to_string();
            }
        }
    }

    // Bare global symbol.
    static BARE_SYMBOL_RE: Lazy<Regex> = Lazy::new(|| re(r"^[A-Za-z_]\w*$"));
    if BARE_SYMBOL_RE.is_match(&operand) {
        if let Some(t) = GLOBAL_TABLE.lock().unwrap().get(&operand) {
            return t.clone();
        }
    }

    "UINT64".to_string()
}

/// Put the address of `name` (a local struct/array, global struct/array,
/// or pointer param) into `reg`. For a pointer param, the spill slot
/// already holds the pointer value we want; for an inline struct/array
/// local we materialise the address as `X29 + offset`; for a non-pointer
/// global we materialise via `ADRP + ADD :lo12:` (not `LDR`, which would
/// dereference).
pub fn emit_struct_base_addr(
    name: &str,
    declared_type: &str,
    reg: &str,
    state: Option<&FuncState>,
    out: &mut Vec<String>,
) {
    let is_pointer = declared_type.starts_with('@') || declared_type == "PTR";
    let off = state.and_then(|st| st.offset_of(name));
    if let Some(off) = off {
        if !is_pointer {
            for ln in aarch64_addsub_imm_lines("ADD", reg, "X29", off, &format!("// &{}", name)) {
                out.push(ln);
            }
            return;
        }
    }
    // Bug #42: non-pointer global (struct/array buffer) — materialise its
    // address, don't dereference.
    if !is_pointer && GLOBAL_TABLE.lock().unwrap().contains_key(name) {
        out.push(format!("\tADRP\t{}, {}", reg, name));
        out.push(format!("\tADD\t{}, {}, :lo12:{}\t// &{} (global)", reg, reg, name, name));
        return;
    }
    emit_load_value(name, reg, state, out, "UINT64");
}

/// Parse a UAGL integer literal's numeric portion (already validated by
/// the caller's regex to be `0x[0-9a-fA-F]+`, `0b[01]+`, or `\d+`) the way
/// Python's `int(s, 0)` would, INCLUDING its rejection of an ambiguous
/// leading zero on a plain decimal literal (e.g. `"007"`), which in the
/// original raises an uncaught `ValueError` that crashes the whole
/// compile. We panic in that same spot for the same reason: this is not a
/// "fall through to a comment" situation in the original, it is a hard
/// stop, and silently doing something different would hide a malformed
/// literal instead of surfacing it.
fn parse_base0_int(s: &str) -> i128 {
    crate::helpers::try_parse_base0(s).unwrap_or_else(|| {
        panic!(
            "invalid literal for int() with base 0: '{s}' — the original Python \
             raises an uncaught ValueError at this exact spot (unlike most other \
             int(val, 0) call sites in the file, this one has no try/except), so \
             this panic mirrors a genuine upstream crash rather than introducing \
             a new failure mode."
        )
    })
}

/// Render an f64 roughly the way Python's `str()`/f-string formatting
/// would for the `// float {f} lo` diagnostic comment: unlike Rust's
/// `Display`, Python always shows at least one fractional digit for a
/// float value (`100.0`, not `100`). This only affects a comment's text,
/// never the emitted opcov des, so exactness beyond this is not pursued
/// (this codebase does not currently use float literals per Bug #6).
fn py_float_str(f: f64) -> String {
    if f.is_nan() {
        return "nan".to_string();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    let s = format!("{}", f);
    if !s.contains('.') && !s.contains('e') && !s.contains('E') {
        format!("{}.0", s)
    } else {
        s
    }
}

/// Emit instruction(s) to load a value into a register. Faithful port of
/// `emit_load_value`; branch numbering in the comments mirrors the
/// original's own (including its incidental duplicate "7." label and a
/// "8. Function call result" branch that is reachable in the original only
/// when branch 6b's identical regex already would have matched — i.e. it
/// is dead code there too, and kept here purely for structural parity).
pub fn emit_load_value(
    val: &str,
    reg: &str,
    state: Option<&FuncState>,
    out: &mut Vec<String>,
    result_type: &str,
) {
    let val = val.trim().trim_end_matches('◇').trim().to_string();
    if val.is_empty() {
        out.push(format!("\tMOV\t{}, XZR", reg));
        return;
    }

    // 1. Immediate integer (decimal, hex 0x..., binary 0b...).
    static INT_RE: Lazy<Regex> = Lazy::new(|| re(r"^(-?)(0x[0-9a-fA-F]+|0b[01]+|\d+)$"));
    if let Some(caps) = INT_RE.captures(&val) {
        let mut n = parse_base0_int(&caps[2]);
        if &caps[1] == "-" {
            n = -n;
        }
        if n == 0 {
            out.push(format!("\tMOV\t{}, XZR", reg));
        } else if (0..=0xFFFFi128).contains(&n) {
            out.push(format!("\tMOV\t{}, #{}", reg, n));
        } else if (-0x8000i128..0).contains(&n) {
            let compl = (!n) & 0xFFFF;
            out.push(format!("\tMOVN\t{}, #{}", reg, compl));
        } else {
            let bits = (n as i64) as u64;
            out.push(format!("\tMOVZ\t{}, #{}", reg, bits & 0xFFFF));
            if (bits >> 16) & 0xFFFF != 0 {
                out.push(format!("\tMOVK\t{}, #{}, LSL #16", reg, (bits >> 16) & 0xFFFF));
            }
            if n > 0xFFFFFFFFi128 && (bits >> 32) & 0xFFFF != 0 {
                out.push(format!("\tMOVK\t{}, #{}, LSL #32", reg, (bits >> 32) & 0xFFFF));
            }
            if n > 0xFFFFFFFFFFFFi128 && (bits >> 48) & 0xFFFF != 0 {
                out.push(format!("\tMOVK\t{}, #{}, LSL #48", reg, (bits >> 48) & 0xFFFF));
            }
        }
        return;
    }

    // 2. Named zero/null/bool.
    if val == "0" || val == "NULL" || val == "false" || val == "VOID" {
        out.push(format!("\tMOV\t{}, XZR", reg));
        return;
    }
    if val == "1" || val == "true" {
        out.push(format!("\tMOV\t{}, #1", reg));
        return;
    }

    // 3. UAGL ─ ─ (add/increment): "a ─ ─ b" means a + b.
    static INCR_RE: Lazy<Regex> = Lazy::new(|| re(r"^(.+?)\s*─\s*─\s*(.+)$"));
    if let Some(caps) = INCR_RE.captures(&val) {
        let lhs_e = caps[1].trim().to_string();
        let rhs_e = caps[2].trim().to_string();
        let lhs_t = operand_aarch64_type(&lhs_e, state);
        emit_load_value(&lhs_e, "X9", state, out, &lhs_t);
        out.push("\tSTR\tX9, [SP, #-16]!".to_string());
        let rhs_t = operand_aarch64_type(&rhs_e, state);
        emit_load_value(&rhs_e, "X10", state, out, &rhs_t);
        out.push("\tLDR\tX9, [SP], #16".to_string());
        out.push(format!("\tADD\t{}, X9, X10", reg));
        return;
    }

    // 4. Outer parens — strip and recurse.
    if val.starts_with('(') && val.ends_with(')') {
        let stripped = strip_outer_parens(&val);
        if stripped != val {
            emit_load_value(&stripped, reg, state, out, result_type);
            return;
        }
    }

    // 5. Array subscript: arr[idx_expr] (idx_expr can be any expression).
    static ARR_SUB_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\[([^\]]+)\]$"));
    if let Some(caps) = ARR_SUB_RE.captures(&val) {
        let arr_name = caps[1].to_string();
        let idx_expr = caps[2].trim().to_string();
        // Special-case `mem[addr]` — raw byte-address dereference for MMIO.
        if arr_name == "mem" {
            emit_load_value(&idx_expr, "X10", state, out, "UINT64");
            let wreg = if let Some(rest) = reg.strip_prefix('X') {
                format!("W{}", rest)
            } else {
                reg.to_string()
            };
            out.push(format!("\tLDR\t{}, [X10]\t// mem[addr] MMIO read 32-bit", wreg));
            return;
        }
        emit_load_value(&idx_expr, "X10", state, out, "UINT64");
        let off = state.and_then(|st| st.offset_of(&arr_name));
        if let Some(off) = off {
            let arr_type = state.unwrap().type_of(&arr_name);
            if arr_type.starts_with('@') || arr_type == "PTR" {
                out.push(format!("\tLDR\tX11, [X29, #{}]\t// load ptr {}", off, arr_name));
            } else {
                for ln in aarch64_addsub_imm_lines("ADD", "X11", "X29", off, "") {
                    out.push(ln);
                }
            }
        } else {
            out.push(format!("\tADRP\tX11, {}", arr_name));
            out.push(format!("\tADD\tX11, X11, :lo12:{}", arr_name));
        }
        let elem_t = if off.is_some() {
            let tname = state.unwrap().type_of(&arr_name);
            if let Some(stripped) = tname.strip_prefix('@') {
                stripped.to_string()
            } else if let Some(idx) = tname.find('[') {
                tname[..idx].to_string()
            } else {
                tname
            }
        } else {
            crate::types::global_array_elem_stride(&arr_name, "INT32").0
        };
        let wreg = if let Some(rest) = reg.strip_prefix('X') {
            format!("W{}", rest)
        } else {
            reg.to_string()
        };
        match elem_t.as_str() {
            "UINT8" | "U8" | "INT8" | "S8" | "BOOL" | "CHAR" => {
                out.push(format!("\tLDRB\t{}, [X11, X10]\t// {}[idx] byte", wreg, arr_name));
            }
            "INT32" | "S32" => {
                out.push(format!("\tLDRSW\t{}, [X11, X10, LSL #2]\t// {}[idx] INT32", reg, arr_name));
            }
            "UINT32" | "U32" => {
                out.push(format!("\tLDR\t{}, [X11, X10, LSL #2]\t// {}[idx] U32", wreg, arr_name));
            }
            _ => {
                out.push(format!("\tLDR\t{}, [X11, X10, LSL #3]\t// {}[idx] 8B", reg, arr_name));
            }
        }
        return;
    }

    // 5b. Unary minus prefix: -expr → load expr, NEG it.
    static PLAIN_INT_RE: Lazy<Regex> = Lazy::new(|| re(r"^-?\d+$"));
    if val.starts_with('-') && !PLAIN_INT_RE.is_match(&val) {
        let inner = val[1..].trim().to_string();
        if !inner.is_empty() {
            emit_load_value(&inner, reg, state, out, result_type);
            out.push(format!("\tNEG\t{}, {}", reg, reg));
            return;
        }
    }

    // 5b'. Bitwise NOT prefix: ~expr → load expr, MVN.
    if let Some(inner) = val.strip_prefix('~') {
        let inner = inner.trim().to_string();
        if !inner.is_empty() {
            emit_load_value(&inner, reg, state, out, result_type);
            out.push(format!("\tMVN\t{}, {}", reg, reg));
            return;
        }
    }

    // 5c. Address-of: &var / &arr[idx] / &obj ─ △ field / &obj ─ △ field[idx].
    if let Some(inner) = val.strip_prefix('&') {
        let inner = inner.trim().to_string();

        static BARE_NAME_RE: Lazy<Regex> = Lazy::new(|| re(r"^\w+$"));
        if BARE_NAME_RE.is_match(&inner) {
            if let Some(off) = state.and_then(|st| st.offset_of(&inner)) {
                for ln in aarch64_addsub_imm_lines("ADD", reg, "X29", off, "") {
                    out.push(ln);
                }
            } else {
                out.push(format!("\tADRP\t{}, {}", reg, inner));
                out.push(format!("\tADD\t{}, {}, :lo12:{}", reg, reg, inner));
            }
            return;
        }

        static ARR_IDX_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\[([^\]]+)\]$"));
        if let Some(caps) = ARR_IDX_RE.captures(&inner) {
            let arr = caps[1].to_string();
            let idx = caps[2].trim().to_string();
            emit_load_value(&idx, "X10", state, out, "UINT64");
            let off = state.and_then(|st| st.offset_of(&arr));
            let elem_t;
            if let Some(off) = off {
                let arr_type = state.unwrap().type_of(&arr);
                if arr_type.starts_with('@') || arr_type == "PTR" {
                    out.push(format!("\tLDR\t{}, [X29, #{}]\t// load ptr {}", reg, off, arr));
                    elem_t = arr_type.strip_prefix('@').unwrap_or("UINT8").to_string();
                } else {
                    for ln in aarch64_addsub_imm_lines("ADD", reg, "X29", off, "") {
                        out.push(ln);
                    }
                    elem_t = if let Some(idx) = arr_type.find('[') {
                        arr_type[..idx].to_string()
                    } else {
                        arr_type
                    };
                }
            } else {
                out.push(format!("\tADRP\t{}, {}", reg, arr));
                out.push(format!("\tADD\t{}, {}, :lo12:{}", reg, reg, arr));
                elem_t = crate::types::global_array_elem_stride(&arr, "INT32").0;
            }
            let log2 = elem_log2(&elem_t);
            if log2 == 0 {
                out.push(format!("\tADD\t{}, {}, X10\t// &{}[idx] byte addr", reg, reg, arr));
            } else {
                out.push(format!(
                    "\tADD\t{}, {}, X10, LSL #{}\t// &{}[idx] {}",
                    reg, reg, log2, arr, elem_t
                ));
            }
            return;
        }

        static FIELD_IDX_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*─\s*△\s*(\w+)\s*\[([^\]]+)\]$"));
        if let Some(caps) = FIELD_IDX_RE.captures(&inner) {
            let obj = caps[1].to_string();
            let field = caps[2].to_string();
            let idx = caps[3].trim().to_string();
            let obj_t = state.map(|st| st.type_of(&obj)).unwrap_or_else(|| "@".to_string());
            let (elem, log2) = struct_field_elem_stride(&obj_t, &field, "INT32");
            let f_off = field_offset(&obj_t, &field);
            emit_struct_base_addr(&obj, &obj_t, reg, state, out);
            if f_off != 0 {
                for ln in aarch64_addsub_imm_lines(
                    "ADD",
                    reg,
                    reg,
                    f_off,
                    &format!("// .{} offset", field),
                ) {
                    out.push(ln);
                }
            }
            emit_load_value(&idx, "X10", state, out, "UINT64");
            out.push(format!(
                "\tADD\t{}, {}, X10, LSL #{}\t// &.{}[idx] {}",
                reg, reg, log2, field, elem
            ));
            return;
        }

        static FIELD_NOIDX_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*─\s*△\s*(\w+)$"));
        if let Some(caps) = FIELD_NOIDX_RE.captures(&inner) {
            let obj = caps[1].to_string();
            let field = caps[2].to_string();
            let obj_t = state.map(|st| st.type_of(&obj)).unwrap_or_else(|| "@".to_string());
            let f_off = field_offset(&obj_t, &field);
            emit_struct_base_addr(&obj, &obj_t, reg, state, out);
            if f_off != 0 {
                for ln in aarch64_addsub_imm_lines("ADD", reg, reg, f_off, &format!("// &.{}", field)) {
                    out.push(ln);
                }
            } else {
                out.push(format!("\t// &.{}  (offset 0)", field));
            }
            return;
        }
    }

    // 5d. Struct field with subscript: obj ─ △ field[idx].
    static FIELD_SUB_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*─\s*△\s*(\w+)\s*\[([^\]]+)\]$"));
    if let Some(caps) = FIELD_SUB_RE.captures(&val) {
        let obj = caps[1].to_string();
        let field = caps[2].to_string();
        let idx = caps[3].trim().to_string();
        let obj_t = state.map(|st| st.type_of(&obj)).unwrap_or_else(|| "@".to_string());
        let (elem, log2) = struct_field_elem_stride(&obj_t, &field, "INT32");
        let f_off = field_offset(&obj_t, &field);
        emit_struct_base_addr(&obj, &obj_t, "X11", state, out);
        if f_off != 0 {
            for ln in aarch64_addsub_imm_lines("ADD", "X11", "X11", f_off, &format!("// .{} offset", field)) {
                out.push(ln);
            }
        }
        emit_load_value(&idx, "X10", state, out, "UINT64");
        let wreg = if let Some(rest) = reg.strip_prefix('X') {
            format!("W{}", rest)
        } else {
            reg.to_string()
        };
        match elem.as_str() {
            "UINT8" | "U8" | "INT8" | "S8" | "BOOL" | "CHAR" => {
                out.push(format!("\tLDRB\t{}, [X11, X10]\t// .{}[idx] byte", wreg, field));
            }
            "INT32" | "S32" => {
                out.push(format!(
                    "\tLDRSW\t{}, [X11, X10, LSL #{}]\t// .{}[idx] INT32",
                    reg, log2, field
                ));
            }
            "UINT32" | "U32" | "FLOAT" => {
                out.push(format!(
                    "\tLDR\t{}, [X11, X10, LSL #{}]\t// .{}[idx] U32",
                    wreg, log2, field
                ));
            }
            _ if log2 > 3 => {
                // Bug #43: struct/large-element field — value-load is
                // impossible (struct doesn't fit in a register, and
                // AArch64 LDR LSL max is #3). Return the address instead.
                out.push(format!(
                    "\tADD\t{}, X11, X10, LSL #{}\t// &.{}[idx] {} (addr)",
                    reg, log2, field, elem
                ));
            }
            _ => {
                out.push(format!("\tLDR\t{}, [X11, X10, LSL #{}]\t// .{}[idx] 8B", reg, log2, field));
            }
        }
        return;
    }

    // 6. Float literal — compare as integer bits.
    if let Ok(f) = val.trim().parse::<f64>() {
        if f.is_finite() || f.is_nan() || f.is_infinite() {
            let bits = (f as f32).to_bits();
            let lo16 = bits & 0xFFFF;
            let hi16 = (bits >> 16) & 0xFFFF;
            out.push(format!("\tMOVZ\tW9, #{:#06x}  // float {} lo", lo16, py_float_str(f)));
            if hi16 != 0 {
                out.push(format!("\tMOVK\tW9, #{:#06x}, LSL #16  // float {} hi", hi16, py_float_str(f)));
            }
            if reg != "X9" && reg != "W9" {
                out.push(format!("\tMOV\t{}, X9  // float {}", reg, py_float_str(f)));
            }
            return;
        }
    }

    // 6b. Function call (must come before binary op so `f ○ a * b` parses
    // as a call with arg `a * b`, not as `(f ○ a) * b`).
    if has_top_level(&val, '○') {
        static CALL_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*○\s*(.*)$"));
        if let Some(caps) = CALL_RE.captures(&val) {
            emit_call(&caps[1], &caps[2], state, out);
            if reg != "X0" {
                out.push(format!("\tMOV\t{}, X0", reg));
            }
            return;
        }
    }

    // 7. Binary operator (precedence-aware, paren-aware, with register
    // save/restore).
    if let Some((lhs_e, op, rhs_e)) = split_binop(&val) {
        let lhs_t = operand_aarch64_type(&lhs_e, state);
        let rhs_t = operand_aarch64_type(&rhs_e, state);
        emit_load_value(&lhs_e, "X9", state, out, &lhs_t);
        out.push("\tSTR\tX9, [SP, #-16]!".to_string());
        emit_load_value(&rhs_e, "X10", state, out, &rhs_t);
        out.push("\tLDR\tX9, [SP], #16".to_string());

        let signed = lhs_t.starts_with("INT") || rhs_t.starts_with("INT") || result_type.starts_with("INT");
        if op == "*" {
            const NARROW_TYPES: &[&str] = &[
                "INT32", "UINT32", "INT16", "UINT16", "INT8", "UINT8", "BOOL", "CHAR", "BYTE",
                "S32", "U32", "S16", "U16", "S8", "U8",
            ];
            let lhs_narrow = NARROW_TYPES.contains(&lhs_t.as_str());
            let rhs_narrow = NARROW_TYPES.contains(&rhs_t.as_str());
            if matches!(result_type, "INT64" | "UINT64" | "S64" | "U64") && lhs_narrow && rhs_narrow {
                if signed {
                    out.push(format!("\tSMULL\t{}, W9, W10", reg));
                } else {
                    out.push(format!("\tUMULL\t{}, W9, W10", reg));
                }
            } else {
                out.push(format!("\tMUL\t{}, X9, X10", reg));
            }
        } else if op == "/" {
            out.push(format!("\t{}\t{}, X9, X10", if signed { "SDIV" } else { "UDIV" }, reg));
        } else if op == "%" {
            let div = if signed { "SDIV" } else { "UDIV" };
            out.push(format!("\t{}\tX11, X9, X10", div));
            out.push(format!("\tMSUB\t{}, X11, X10, X9", reg));
        } else if op == "<<" {
            out.push(format!("\tLSL\t{}, X9, X10", reg));
        } else if op == ">>" {
            out.push(format!("\t{}\t{}, X9, X10", if signed { "ASR" } else { "LSR" }, reg));
        } else {
            let opcode = match op.as_str() {
                "+" => "ADD",
                "-" => "SUB",
                "&" => "AND",
                "|" => "ORR",
                "^" => "EOR",
                _ => "ADD",
            };
            out.push(format!("\t{}\t{}, X9, X10", opcode, reg));
        }
        return;
    }

    // 7. Arrow/field (multi-level + optional trailing subscript):
    //    obj ─ △ f1 [─ △ f2 [─ △ f3 …]] [\[idx\]]
    static MULTI_FIELD_RE: Lazy<Regex> =
        Lazy::new(|| re(r"^(\w+)((?:\s*─\s*△\s*\w+)+)(?:\s*\[([^\]]+)\])?$"));
    static FIELD_NAME_RE: Lazy<Regex> = Lazy::new(|| re(r"─\s*△\s*(\w+)"));
    if let Some(caps) = MULTI_FIELD_RE.captures(&val) {
        let obj = caps[1].to_string();
        let fields_raw = caps[2].to_string();
        let idx = caps.get(3).map(|m| m.as_str().to_string());
        let obj_t = state.map(|st| st.type_of(&obj)).unwrap_or_else(|| "@".to_string());
        let mut cur_t = obj_t.trim_start_matches('@').trim().to_string();
        if let Some(i) = cur_t.find('[') {
            cur_t = cur_t[..i].to_string();
        }
        emit_struct_base_addr(&obj, &obj_t, reg, state, out);
        let mut leaf_t = cur_t.clone();
        for fcaps in FIELD_NAME_RE.captures_iter(&fields_raw) {
            let fld = fcaps[1].to_string();
            let f_off = field_offset(&cur_t, &fld);
            if f_off != 0 {
                for ln in aarch64_addsub_imm_lines("ADD", reg, reg, f_off, &format!("// .{}", fld)) {
                    out.push(ln);
                }
            }
            let ftype = field_type(&cur_t, &fld);
            let mut inner = ftype.trim_start_matches('@').trim().to_string();
            if let Some(i) = inner.find('[') {
                inner = inner[..i].to_string();
            }
            cur_t = inner.clone();
            leaf_t = inner;
        }
        let wreg = if let Some(rest) = reg.strip_prefix('X') {
            format!("W{}", rest)
        } else {
            reg.to_string()
        };
        if let Some(idx) = idx {
            emit_load_value(idx.trim(), "X10", state, out, "UINT64");
            let log2 = elem_log2(&leaf_t);
            match leaf_t.as_str() {
                "UINT8" | "U8" | "INT8" | "S8" | "BOOL" | "CHAR" => {
                    out.push(format!("\tLDRB\t{}, [{}, X10]", wreg, reg));
                }
                "INT32" | "S32" => {
                    out.push(format!("\tLDRSW\t{}, [{}, X10, LSL #{}]", reg, reg, log2));
                }
                "UINT32" | "U32" | "FLOAT" => {
                    out.push(format!("\tLDR\t{}, [{}, X10, LSL #{}]", wreg, reg, log2));
                }
                _ => {
                    out.push(format!("\tLDR\t{}, [{}, X10, LSL #{}]", reg, reg, log2));
                }
            }
        } else {
            match leaf_t.as_str() {
                "UINT8" | "U8" | "INT8" | "S8" | "BOOL" | "CHAR" => {
                    out.push(format!("\tLDRB\t{}, [{}]", wreg, reg));
                }
                "INT32" | "S32" => {
                    out.push(format!("\tLDRSW\t{}, [{}]", reg, reg));
                }
                "UINT32" | "U32" | "FLOAT" => {
                    out.push(format!("\tLDR\t{}, [{}]", wreg, reg));
                }
                _ => {
                    out.push(format!("\tLDR\t{}, [{}]", reg, reg));
                }
            }
        }
        return;
    }

    // 8. Function call result: func ○ args. Reachable in the original only
    // when branch 6b's identical regex already matched (in which case it
    // would already have returned) — i.e. genuinely unreachable there too.
    // Kept for structural/textual parity with the source being ported.
    static CALL_RESULT_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*○\s*(.*)$"));
    if let Some(caps) = CALL_RESULT_RE.captures(&val) {
        emit_call(&caps[1], &caps[2], state, out);
        if reg != "X0" {
            out.push(format!("\tMOV\t{}, X0", reg));
        }
        return;
    }

    // 9. Local variable (params are spilled into locals at function entry).
    if let Some(st) = state {
        if let Some(off) = st.offset_of(&val) {
            let ltype = st.type_of(&val);
            if matches!(ltype.as_str(), "INT32" | "S32") && reg.starts_with('X') {
                out.push(format!("\tLDRSW\t{}, [X29, #{}]\t// signed-load INT32 local", reg, off));
            } else {
                out.push(format!("\tLDR\t{}, [X29, #{}]", reg, off));
            }
            return;
        }
    }

    // 11. Global symbol (ADRP + LDR).
    static SYMBOL_RE: Lazy<Regex> = Lazy::new(|| re(r"^[A-Za-z_]\w*$"));
    if SYMBOL_RE.is_match(&val) {
        out.push(format!("\tADRP\tX9, {}", val));
        out.push(format!("\tADD\tX9, X9, :lo12:{}", val));
        out.push(format!("\tLDR\t{}, [X9]", reg));
        return;
    }

    // Fallback.
    crate::diagnostics::record(|| format!("Unresolved expression: {val}"));
    out.push(format!("\t// unresolved: {}", val));
}
