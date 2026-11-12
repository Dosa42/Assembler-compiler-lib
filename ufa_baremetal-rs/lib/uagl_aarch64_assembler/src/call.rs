//! Function-call codegen — a faithful port of `_split_call_args`,
//! `emit_call`, `emit_comparison`, and `emit_scope`.

use crate::expr::emit_load_value;
use crate::func_state::FuncState;
use crate::helpers::{decode_uagl_string, parse_condition};
use crate::tables::{cmp_to_branch, sysreg_map, GLOBAL_TABLE};
use once_cell::sync::Lazy;
use regex::Regex;

fn re(pat: &str) -> Regex {
    Regex::new(pat).unwrap_or_else(|e| panic!("bad regex {pat:?}: {e}"))
}

/// Split call args on `─` at paren+bracket depth 0 only. UAGL field-access
/// subexpressions like `obj ─ △ field` stay together (a `─` immediately
/// followed by `△`, ignoring whitespace, is not treated as a separator).
pub fn split_call_args(args_raw: &str) -> Vec<String> {
    let args_raw = args_raw
        .trim_end()
        .trim_end_matches('◇')
        .trim_end();
    let chars: Vec<char> = args_raw.chars().collect();
    let n = chars.len();
    let mut parts: Vec<String> = Vec::new();
    let mut paren = 0i64;
    let mut bracket = 0i64;
    let mut cur: Vec<char> = Vec::new();
    let mut i = 0usize;
    while i < n {
        let c = chars[i];
        match c {
            '(' => {
                paren += 1;
                cur.push(c);
                i += 1;
            }
            ')' => {
                paren -= 1;
                cur.push(c);
                i += 1;
            }
            '[' => {
                bracket += 1;
                cur.push(c);
                i += 1;
            }
            ']' => {
                bracket -= 1;
                cur.push(c);
                i += 1;
            }
            '─' if paren == 0 && bracket == 0 => {
                let rest: String = chars[i + 1..].iter().collect::<String>();
                let rest = rest.trim_start();
                if rest.starts_with('△') {
                    cur.push(c);
                    i += 1;
                    continue;
                }
                let piece: String = cur.iter().collect::<String>().trim().to_string();
                if !piece.is_empty() {
                    parts.push(piece);
                }
                cur.clear();
                i += 1;
                continue;
            }
            _ => {
                cur.push(c);
                i += 1;
            }
        }
    }
    let last: String = cur.iter().collect::<String>().trim().to_string();
    if !last.is_empty() {
        parts.push(last);
    }
    parts
}

/// Emit `BL`/`BLR` for a function call, loading args into X0..X7. Handles
/// the `__memcpy`/`__memset`/`__asm__` intrinsics and indirect
/// (function-pointer) calls.
pub fn emit_call(func: &str, args_raw: &str, state: Option<&FuncState>, out: &mut Vec<String>) {
    let args = split_call_args(args_raw);

    // Bug #56: __memcpy(dst, src, n) intrinsic — inline byte-copy loop
    // instead of `BL memcpy`, so bare-metal doesn't need a libc memcpy.
    if func == "__memcpy" {
        if args.len() != 3 {
            crate::diagnostics::record(|| format!("__memcpy expects 3 arguments, got {}", args.len()));
            out.push(format!("\t// __memcpy expects (dst, src, n), got {} args", args.len()));
            return;
        }
        emit_load_value(&args[0], "X0", state, out, "UINT64");
        emit_load_value(&args[1], "X1", state, out, "UINT64");
        emit_load_value(&args[2], "X2", state, out, "UINT64");
        let tag = state
            .map(|st| st.next_label("memcpy"))
            .unwrap_or_else(|| ".memcpy_tmp".to_string());
        out.push(format!("{}_top:", tag));
        out.push(format!("\tCBZ\tX2, {}_end", tag));
        out.push("\tLDRB\tW9, [X1], #1".to_string());
        out.push("\tSTRB\tW9, [X0], #1".to_string());
        out.push("\tSUB\tX2, X2, #1".to_string());
        out.push(format!("\tB\t{}_top", tag));
        out.push(format!("{}_end:", tag));
        return;
    }

    // Bug #57: __asm__("raw asm") inline — emit the literal asm directly.
    if func == "__asm__" {
        if args.len() != 1 || !args[0].trim().starts_with('"') {
            crate::diagnostics::record(|| "__asm__ expects a single string literal".to_string());
            out.push(format!("\t// __asm__ expects single string literal, got: {:?}", args));
            return;
        }
        let quoted = args[0].trim();
        let raw = crate::helpers::drop_first_last_char(quoted);
        let bytes = decode_uagl_string(&raw);
        let text = String::from_utf8_lossy(&bytes).to_string();
        for line in text.split('\n') {
            let line = line.trim();
            if !line.is_empty() {
                out.push(format!("\t{}", line));
            }
        }
        return;
    }

    // Bug #56b: __memset(dst, byte_val, n) — inline byte-set loop.
    if func == "__memset" {
        if args.len() != 3 {
            crate::diagnostics::record(|| format!("__memset expects 3 arguments, got {}", args.len()));
            out.push(format!("\t// __memset expects (dst, byte_val, n), got {} args", args.len()));
            return;
        }
        emit_load_value(&args[0], "X0", state, out, "UINT64");
        emit_load_value(&args[1], "X9", state, out, "UINT64");
        emit_load_value(&args[2], "X2", state, out, "UINT64");
        let tag = state
            .map(|st| st.next_label("memset"))
            .unwrap_or_else(|| ".memset_tmp".to_string());
        out.push(format!("{}_top:", tag));
        out.push(format!("\tCBZ\tX2, {}_end", tag));
        out.push("\tSTRB\tW9, [X0], #1".to_string());
        out.push("\tSUB\tX2, X2, #1".to_string());
        out.push(format!("\tB\t{}_top", tag));
        out.push(format!("{}_end:", tag));
        return;
    }

    // Bug #58: function-pointer call. If `func` names a local or global
    // variable (rather than a function symbol), emit an indirect BLR.
    let is_indirect = if let Some(st) = state {
        if st.offset_of(func).is_some() {
            true
        } else {
            GLOBAL_TABLE.lock().unwrap().contains_key(func)
        }
    } else {
        GLOBAL_TABLE.lock().unwrap().contains_key(func)
    };

    if is_indirect {
        // Resolve target into X9 BEFORE loading args (X0..X7), since arg
        // loads may clobber X9 themselves; spill it across the stack.
        emit_load_value(func, "X9", state, out, "UINT64");
        out.push("\tSTR\tX9, [SP, #-16]!\t// spill fn ptr across arg loads".to_string());
        for (i, arg) in args.iter().take(8).enumerate() {
            emit_load_value(arg, &format!("X{}", i), state, out, "UINT64");
        }
        out.push("\tLDR\tX9, [SP], #16\t// reload fn ptr".to_string());
        out.push(format!("\tBLR\tX9\t// indirect call via {}", func));
        return;
    }

    for (i, arg) in args.iter().take(8).enumerate() {
        emit_load_value(arg, &format!("X{}", i), state, out, "UINT64");
    }

    match func {
        "println" => out.push("\tBL\tuagl_println".to_string()),
        "print" => out.push("\tBL\tuagl_print".to_string()),
        _ => out.push(format!("\tBL\t{}", func)),
    }
}

/// Emit compare + branch instruction. Not currently called from the
/// generator (control-flow codegen uses `AArch64Generator::emit_simple_cmp`
/// instead), but ported for parity with the original public surface.
#[allow(dead_code)]
pub fn emit_comparison(cond: &str, true_label: &str, state: Option<&FuncState>, out: &mut Vec<String>) {
    let (lhs, op, rhs) = parse_condition(cond);
    emit_load_value(&lhs, "X9", state, out, "UINT64");

    if rhs == "0" && (op == "╱" || op == "!=") {
        out.push(format!("\tCBNZ\tX9, {}", true_label));
        return;
    }
    if rhs == "0" && (op == "△" || op == "==") {
        out.push(format!("\tCBZ\tX9, {}", true_label));
        return;
    }

    emit_load_value(&rhs, "X10", state, out, "UINT64");
    out.push("\tCMP\tX9, X10".to_string());
    let branch = cmp_to_branch(&op);
    out.push(format!("\t{}\t{}", branch, true_label));
}

/// `○ args ◇` — MRS/MSR/cache-op/SP-set/SP-read. Faithfully preserves an
/// upstream quirk: the store-offset checks use Python truthiness (`if
/// off:`) rather than an explicit `is not None` test, so a destination
/// local at stack offset **0** (the first local allocated in a function)
/// is treated the same as "not a local" and falls through to the
/// global/register fallback path instead of `STR X9, [X29, #0]`. This
/// mirrors the source exactly rather than "fixing" it — see the port
/// notes.
pub fn emit_scope(args: &str, state: Option<&FuncState>, out: &mut Vec<String>) {
    let args = args.trim();

    // MRS: ○ dst ─ SYSREG   /   MSR: ○ SYSREG ─ src
    static MRS_MSR_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*─\s*(\w+)$"));
    if let Some(caps) = MRS_MSR_RE.captures(args) {
        let dst = &caps[1];
        let sysreg = &caps[2];
        if let Some(reg_name) = sysreg_map(&sysreg.to_uppercase()) {
            out.push(format!("\tMRS\tX9, {}", reg_name));
            let off = state.and_then(|st| st.offset_of(dst));
            let truthy = off.map(|n| n != 0).unwrap_or(false);
            if truthy {
                out.push(format!("\tSTR\tX9, [X29, #{}]", off.unwrap()));
            } else {
                out.push(format!("\tMOV\tX0, X9\t// {} = {}", dst, sysreg));
            }
            return;
        }
        if let Some(reg_name) = sysreg_map(&dst.to_uppercase()) {
            emit_load_value(sysreg, "X9", state, out, "UINT64");
            out.push(format!("\tMSR\t{}, X9", reg_name));
            return;
        }
    }

    // SP set: ○ stack_base ◇ (stack pointer init).
    static BARE_WORD_RE: Lazy<Regex> = Lazy::new(|| re(r"^\w+$"));
    if BARE_WORD_RE.is_match(args) {
        let varname = args;
        let off = state.and_then(|st| st.offset_of(varname));
        let truthy = off.map(|n| n != 0).unwrap_or(false);
        if truthy {
            out.push(format!("\tLDR\tX9, [X29, #{}]", off.unwrap()));
        } else {
            out.push(format!("\tADRP\tX9, {}", varname));
            out.push(format!("\tADD\tX9, X9, :lo12:{}", varname));
        }
        out.push(format!("\tMOV\tSP, X9\t// set SP from {}", varname));
        return;
    }

    // Cache flush: ○ addr ◇ — unreachable in the original (identical
    // `^\w+$` shape already handled, and returned from, just above); kept
    // for structural parity.
    static ADDR_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)$"));
    if let Some(caps) = ADDR_RE.captures(args) {
        let addr = &caps[1];
        emit_load_value(addr, "X9", state, out, "UINT64");
        out.push("\tDC\tCIVAC, X9".to_string());
        return;
    }

    out.push(format!("\t// SCOPE: {}", args));
}
