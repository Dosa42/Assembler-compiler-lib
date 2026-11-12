//! seL4 C type and subsystem mappings for UAGL v3.

pub type GVector = (f64, f64, f64, f64);

pub const EXACT_MAP: &[(&str, &str)] = &[
    ("uint8_t", "UINT8"),
    ("uint16_t", "UINT16"),
    ("uint32_t", "UINT32"),
    ("uint64_t", "UINT64"),
    ("int8_t", "INT8"),
    ("int16_t", "INT16"),
    ("int32_t", "INT32"),
    ("int64_t", "INT64"),
    ("uintptr_t", "UINT64"),
    ("intptr_t", "INT64"),
    ("size_t", "UINT64"),
    ("ssize_t", "INT64"),
    ("char", "CHAR"),
    ("void", "VOID"),
    ("bool", "BOOL"),
    ("_Bool", "BOOL"),
    ("float", "FLOAT"),
    ("double", "DOUBLE"),
    ("word_t", "UINT64"),
    ("sword_t", "INT64"),
    ("vptr_t", "PTR"),
    ("pptr_t", "PTR"),
    ("paddr_t", "UINT64"),
    ("vaddr_t", "UINT64"),
    ("region_t", "UINT64"),
    ("seL4_Word", "UINT64"),
    ("seL4_Uint8", "UINT8"),
    ("seL4_Uint16", "UINT16"),
    ("seL4_Uint32", "UINT32"),
    ("seL4_Uint64", "UINT64"),
    ("seL4_Bool", "BOOL"),
    ("seL4_Char", "CHAR"),
    ("exception_t", "INT32"),
    ("bool_t", "BOOL"),
    ("tcb_state_t", "INT32"),
    ("prio_t", "UINT8"),
    ("dom_t", "UINT8"),
    ("irq_t", "UINT32"),
    ("ticks_t", "UINT64"),
    ("time_t", "UINT64"),
    ("cpu_id_t", "UINT32"),
    ("node_id_t", "UINT32"),
    ("void*", "PTR"),
];

pub const SEL4_MACROS: &[(&str, &str)] = &[
    ("BOOT_CODE", ""),
    ("BOOT_BSS", ""),
    ("BOOT_RODATA", ""),
    ("NORETURN", ""),
    ("CONST", ""),
    ("PURE", ""),
    ("VISIBLE", ""),
    ("FORCE_INLINE", ""),
    ("NO_INLINE", ""),
    ("ALIGN", ""),
    ("PACKED", ""),
    ("WEAK", ""),
    ("compile_assert", "/* STATIC_ASSERT */"),
    ("likely", ""),
    ("unlikely", ""),
    ("assert", "/* ASSERT */"),
    ("fail", "/* FAIL */"),
    ("NODE_STATE", ""),
    ("UP_STATE_DEFINE", "·"),
    ("SMP_STATE_DEFINE", "·"),
    ("NODE_STATE_ON_CORE", ""),
];

pub const PTR_SUFFIXES: &[&str] = &["_t *", "_t*", "* "];

pub const SUBSYSTEM_GVECTORS: &[(&str, GVector)] = &[
    ("thread", (0.8, 0.3, 0.9, 0.6)),
    ("boot", (0.9, 0.4, 0.7, 0.8)),
    ("cspace", (0.7, 0.2, 0.8, 0.5)),
    ("faulthandler", (0.6, 0.1, 0.7, 0.4)),
    ("sporadic", (0.5, 0.3, 0.6, 0.4)),
    ("syscall", (0.8, 0.4, 0.8, 0.7)),
    ("endpoint", (0.7, 0.3, 0.7, 0.5)),
    ("tcb", (0.8, 0.2, 0.8, 0.6)),
    ("interrupt", (0.9, 0.1, 0.6, 0.7)),
    ("objecttype", (0.6, 0.4, 0.7, 0.4)),
    ("statedata", (0.7, 0.2, 0.8, 0.5)),
    ("ipc", (0.7, 0.3, 0.8, 0.6)),
    ("vspace", (0.6, 0.2, 0.7, 0.5)),
    ("default", (0.6, 0.3, 0.6, 0.4)),
];

/// Map a C type string to the original converter's UAGL v3 type string.
pub fn map_type(c_type: &str) -> String {
    let strip = |ch: char| ch.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&ch);
    let mut cleaned = c_type.trim_matches(strip).to_owned();
    for qualifier in [
        "const ",
        "volatile ",
        "static ",
        "extern ",
        "CONST ",
        "PURE ",
        "restrict ",
        "__restrict ",
        "BOOT_DATA ",
        "BOOT_BSS ",
        "BOOT_RODATA ",
    ] {
        cleaned = cleaned.replace(qualifier, "");
    }
    let mut name = cleaned.trim_matches(strip);
    let mut pointer = false;
    while let Some(base) = name.strip_suffix('*') {
        pointer = true;
        name = base.trim_matches(strip);
    }

    let base = if let Some((_, mapped)) = EXACT_MAP.iter().find(|(c, _)| *c == name) {
        mapped
    } else {
        match name {
            "unsigned int" | "unsigned" => "UINT32",
            "signed int" | "int" => "INT32",
            "unsigned long" | "unsigned long long" => "UINT64",
            "long" | "long long" | "signed long" => "INT64",
            "unsigned short" => "UINT16",
            "unsigned char" => "UINT8",
            _ if name.starts_with("struct ") => name[7..].trim_matches(strip),
            _ if name.starts_with("enum ") => "INT32",
            "" if pointer => return "PTR".to_owned(),
            "" => "VOID",
            _ => name,
        }
    };
    if pointer {
        format!("@{base}")
    } else {
        base.to_owned()
    }
}

pub fn map_array_type(c_type: &str, size: &str) -> String {
    let base = map_type(c_type);
    format!("{}[{size}]", base.strip_prefix('@').unwrap_or(&base))
}

pub fn get_gvector(filename: &str) -> GVector {
    let name = filename.to_lowercase();
    SUBSYSTEM_GVECTORS
        .iter()
        .find(|(key, _)| name.contains(key))
        .map_or((0.6, 0.3, 0.6, 0.4), |(_, vector)| *vector)
}

pub fn kappa(&(g1, g2, g3, g4): &GVector) -> f64 {
    0.34 * g1.max(0.0) + 0.20 * g2.max(0.0) + 0.32 * g3.max(0.0) + 0.14 * g4
}

pub fn tessa_class(k: f64) -> &'static str {
    if k < 0.15 {
        "ENEMY"
    } else if k < 0.45 {
        "TROJAN"
    } else if k < 0.55 {
        "PARASITE"
    } else if k < 0.71 {
        "BENEFACTOR"
    } else {
        "INERT"
    }
}
