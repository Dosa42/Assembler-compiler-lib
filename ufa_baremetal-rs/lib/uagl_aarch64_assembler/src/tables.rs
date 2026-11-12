//! Module-level mutable state and static lookup tables.
//!
//! The Python source keeps three module-level `dict`s that accumulate across
//! every `assemble_uagl()` call made within one process (this is a
//! deliberately documented invariant: "Single Python process invariant
//! preserved... `_STRUCT_TABLE` accumulates across files" — see
//! `UFA_BAREMETAL_CANON.md` §2.9). `assemble_batch` compiles multiple files
//! concurrently via a thread pool, so in Rust these need to be shared,
//! thread-safe, process-lifetime globals rather than per-call state. We use
//! `Mutex`-guarded `HashMap`s behind `once_cell::sync::Lazy` statics, which
//! is the direct equivalent of "module dict mutated under the GIL" for a
//! multi-threaded Rust binary.

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Mutex;

/// `_STRUCT_TABLE`: struct name -> {field name -> declared field type}.
/// Insertion order matters (struct layout walks fields in declaration
/// order), so each struct's fields are stored as an ordered `Vec` of pairs
/// rather than a `HashMap`, wrapped in an outer `HashMap` keyed by struct
/// name (Python dicts preserve insertion order since 3.7, which is exactly
/// what `_compute_struct_layout` relies on when it iterates `fields.items()`).
pub static STRUCT_TABLE: Lazy<Mutex<HashMap<String, Vec<(String, String)>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// `_GLOBAL_TABLE`: global symbol name -> declared element type.
pub static GLOBAL_TABLE: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// `_STRUCT_LAYOUTS`: cached struct name -> ({field -> byte offset}, total size).
pub static STRUCT_LAYOUTS: Lazy<Mutex<HashMap<String, (HashMap<String, i64>, i64)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Test-only / embedding-only helper: clears all accumulated global state.
/// The Python module never does this (a fresh `_STRUCT_TABLE` only exists
/// per-process), but our test harness runs many independent programs in one
/// process, so each test that wants file-local isolation calls this first.
pub fn reset_all_tables() {
    STRUCT_TABLE.lock().unwrap().clear();
    GLOBAL_TABLE.lock().unwrap().clear();
    STRUCT_LAYOUTS.lock().unwrap().clear();
}

/// `TYPE_TO_REG`: UAGL type -> AArch64 register size prefix.
pub fn type_to_reg(utype: &str) -> &'static str {
    match utype {
        "UINT8" | "INT8" => "W",
        "UINT16" | "INT16" => "W",
        "UINT32" | "INT32" => "W",
        "UINT64" | "INT64" => "X",
        "FLOAT" => "S",
        "DOUBLE" => "D",
        "BOOL" | "CHAR" => "W",
        "PTR" | "VOID" => "X",
        "GVector" => "X",
        _ => "X",
    }
}

/// `SYSREG_MAP`: known AArch64 system registers (identity map; the lookup
/// itself acts as an allow-list gate against arbitrary MRS/MSR targets).
pub fn sysreg_map(upper: &str) -> Option<&'static str> {
    const NAMES: &[&str] = &[
        "MPIDR_EL1",
        "CPACR_EL1",
        "SCTLR_EL1",
        "DAIF",
        "VBAR_EL1",
        "ELR_EL1",
        "SPSR_EL1",
        "SP_EL0",
        "SP_EL1",
        "TTBR0_EL1",
        "TTBR1_EL1",
        "TCR_EL1",
        "ESR_EL1",
        "FAR_EL1",
        "TPIDR_EL0",
        "TPIDR_EL1",
        "CNTPCT_EL0",
        "CNTP_CTL_EL0",
    ];
    NAMES.iter().find(|&&n| n == upper).copied()
}

/// `CMP_TO_BRANCH`: UAGL comparison operator -> AArch64 branch condition.
pub fn cmp_to_branch(op: &str) -> &'static str {
    match op {
        ">" => "B.GT",
        "<" => "B.LT",
        ">=" => "B.GE",
        "<=" => "B.LE",
        "△" => "B.EQ",
        "╱" => "B.NE",
        "∧" => "B.NE",
        "∨" => "B.NE",
        _ => "B.NE",
    }
}
