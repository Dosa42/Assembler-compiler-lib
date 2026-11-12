//! Per-function code generator state — a faithful port of the `FuncState`
//! dataclass and the free function `reg_for`.

use crate::tables::{type_to_reg, GLOBAL_TABLE};
use crate::types::type_size;
use std::cell::Cell;
use std::collections::HashMap;

/// `label_counter` is `Cell<i64>` rather than a plain field so that
/// `next_label` can be called through a shared `&FuncState`. In the
/// original, the expression-codegen call graph (`emit_load_value`,
/// `emit_call`, `_operand_aarch64_type`, `_emit_struct_base_addr`,
/// `emit_comparison`, `emit_scope`) only ever *reads* `FuncState`
/// (`.params`, `.locals` via `offset_of`/`type_of`) with the single
/// exception of `emit_call`'s `__memcpy`/`__memset` builtins bumping the
/// label counter. Modeling just that one field with interior mutability
/// lets `Option<&FuncState>` (a `Copy` value) flow freely through that
/// whole read-mostly call graph instead of threading `Option<&mut _>`
/// reborrows through dozens of recursive call sites. `alloc_local` (which
/// does mutate `locals`/`stack_size`) is only ever called from the
/// top-level generator methods, which hold the `FuncState` by value and
/// use plain `&mut self` there.
#[derive(Debug, Clone)]
pub struct FuncState {
    pub name: String,
    /// [(param_name, param_type), ...] in declaration order.
    pub params: Vec<(String, String)>,
    #[allow(dead_code)]
    pub ret_type: String,
    /// name -> (stack_offset, type)
    pub locals: HashMap<String, (i64, String)>,
    pub stack_size: i64,
    pub label_counter: Cell<i64>,
    /// Index of the placeholder line in `out[]` for the prologue `SUB SP`.
    /// -1 means "not yet allocated" (matches the Python default).
    pub stack_alloc_idx: i64,
}

impl FuncState {
    pub fn new(name: String, params: Vec<(String, String)>, ret_type: String) -> Self {
        FuncState {
            name,
            params,
            ret_type,
            locals: HashMap::new(),
            stack_size: 0,
            label_counter: Cell::new(0),
            stack_alloc_idx: -1,
        }
    }

    pub fn next_label(&self, prefix: &str) -> String {
        let next = self.label_counter.get() + 1;
        self.label_counter.set(next);
        format!(".{}_{}{}", self.name, prefix, next)
    }

    /// Allocate a stack slot, returning the offset from SP/X29. Offsets
    /// start at 0 and grow upward, each slot rounded up to 8-byte alignment.
    pub fn alloc_local(&mut self, name: &str, utype: &str) -> i64 {
        let size = type_size(utype);
        let aligned = (size + 7) & !7;
        let offset = self.stack_size;
        self.stack_size += aligned;
        self.locals
            .insert(name.to_string(), (offset, utype.to_string()));
        offset
    }

    pub fn offset_of(&self, name: &str) -> Option<i64> {
        self.locals.get(name).map(|(off, _)| *off)
    }

    /// Bug #41: globals weren't visible to `type_of`, so `&global ─ △ field`
    /// accesses fell through to the UINT64 -> INT32 stride fallback.
    pub fn type_of(&self, name: &str) -> String {
        if let Some((_, t)) = self.locals.get(name) {
            return t.clone();
        }
        if let Some(t) = GLOBAL_TABLE.lock().unwrap().get(name) {
            return t.clone();
        }
        "UINT64".to_string()
    }
}

/// Map a UAGL variable name to an AArch64 register or memory reference.
/// Note: `size` is accepted (matching the original signature) but unused —
/// the Python original never references it either.
#[allow(dead_code, unused_variables)]
pub fn reg_for(name: &str, state: &FuncState, size: &str) -> String {
    for (i, (pname, ptype)) in state.params.iter().enumerate() {
        if pname == name {
            let prefix = type_to_reg(ptype);
            return format!("{}{}", prefix, i);
        }
    }
    if let Some(off) = state.offset_of(name) {
        return format!("[X29, #{}]", off);
    }
    format!("// global {}", name)
}
