//! Linux kernel macro translations from the original C preprocessor.

use crate::type_map::{map_type, GVector};
use regex::{Captures, Regex};
use std::sync::LazyLock;

pub static LINUX_STRIP_ATTRS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b(__init|__exit|__initdata|__exitdata|__initconst|__exitconst|__initcall|__pure|__cold|__hot|__noinline|__always_inline|__must_check|__used|__unused|__packed|__aligned\(\d+\)|__weak|__noreturn|__printf\(\d+,\s*\d+\)|__deprecated|__rcu|__percpu|__iomem|__user|__kernel|__force|__safe|noinline|notrace|EXPORT_SYMBOL(?:_GPL)?\s*\([^)]+\)|MODULE_LICENSE\s*\([^)]+\)|MODULE_AUTHOR\s*\([^)]+\)|MODULE_DESCRIPTION\s*\([^)]+\)|MODULE_VERSION\s*\([^)]+\)|MODULE_ALIAS\s*\([^)]+\)|MODULE_FIRMWARE\s*\([^)]+\))\b"#).unwrap()
});
pub static LINUX_LIKELY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)\b(likely|unlikely)\s*\((.+?)\)"#).unwrap());
pub static LINUX_BARRIER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b(mb|rmb|wmb|smp_mb|smp_rmb|smp_wmb|barrier)\s*\(\s*\)\s*;"#).unwrap()
});
pub static LINUX_ISB_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bisb\s*\(\s*\)\s*;"#).unwrap());
pub static LINUX_DSB_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bdsb\s*\(\s*(sy|st|ld)?\s*\)\s*;"#).unwrap());
pub static MODULE_INIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"module_init\s*\(\s*(\w+)\s*\)\s*;"#).unwrap());
pub static MODULE_EXIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"module_exit\s*\(\s*(\w+)\s*\)\s*;"#).unwrap());
pub static PER_CPU_DEFINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"DEFINE_PER_CPU\s*\(\s*(.+?)\s*,\s*(\w+)\s*\)\s*;"#).unwrap());
pub static PER_CPU_ACCESS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"per_cpu\s*\(\s*(\w+)\s*,\s*(\w+)\s*\)"#).unwrap());
pub static THIS_CPU_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"this_cpu_(?:read|write|add|sub|inc|dec)\s*\(\s*(\w+)\s*(?:,\s*(.+?))?\s*\)"#)
        .unwrap()
});
pub static SPIN_LOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"spin_lock(?:_irqsave)?\s*\(\s*&?(\w+)(?:\s*,\s*(\w+))?\s*\)\s*;"#).unwrap()
});
pub static SPIN_UNLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"spin_unlock(?:_irqrestore)?\s*\(\s*&?(\w+)(?:\s*,\s*(\w+))?\s*\)\s*;"#).unwrap()
});
pub static DEFINE_SPINLOCK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"DEFINE_SPINLOCK\s*\(\s*(\w+)\s*\)\s*;"#).unwrap());
pub static RCU_READ_LOCK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"rcu_read_lock\s*\(\s*\)\s*;"#).unwrap());
pub static RCU_READ_UNLOCK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"rcu_read_unlock\s*\(\s*\)\s*;"#).unwrap());
pub static RCU_DEREF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"rcu_dereference\s*\(\s*(\w+)\s*\)"#).unwrap());
pub static LIST_FOR_EACH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"list_for_each_entry(?:_safe)?\s*\(\s*(\w+)\s*,\s*(?:(\w+)\s*,\s*)?(\w+)\s*,\s*(\w+)\s*\)"#).unwrap()
});
pub static LIST_ADD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"list_add(?:_tail)?\s*\(\s*&(\w+)\s*(?:\.\s*(\w+))?\s*,\s*&(\w+)\s*\)\s*;"#)
        .unwrap()
});
pub static LIST_DEL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"list_del\s*\(\s*&?(\w+)(?:\.(\w+))?\s*\)\s*;"#).unwrap());
pub static INIT_LIST_HEAD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"INIT_LIST_HEAD\s*\(\s*&?(\w+)(?:\.(\w+))?\s*\)\s*;"#).unwrap());
pub static LIST_HEAD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:LIST_HEAD|HLIST_HEAD)\s*\(\s*(\w+)\s*\)"#).unwrap());
pub static CONTAINER_OF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"container_of\s*\(\s*(\w+)\s*,\s*struct\s+(\w+)\s*,\s*(\w+)\s*\)"#).unwrap()
});
pub static KMALLOC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"kmalloc\s*\(\s*sizeof\s*\(\s*\*?(\w+)\s*\)\s*,\s*\w+\s*\)"#).unwrap()
});
pub static KFREE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"kfree\s*\(\s*(\w+)\s*\)\s*;"#).unwrap());
pub static KZALLOC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"kzalloc\s*\(\s*sizeof\s*\(\s*\*?(\w+)\s*\)\s*,\s*\w+\s*\)"#).unwrap()
});
pub static FOR_EACH_CPU_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"for_each_(?:possible|online|present)_cpu\s*\(\s*(\w+)\s*\)"#).unwrap()
});
pub static ASM_VOLATILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)asm\s+volatile\s*\(\s*"([^"]+)"(?:\s*:\s*([^:)]*?))?(?:\s*:\s*([^:)]*?))?(?:\s*:\s*([^)]*?))?\s*\)"#).unwrap()
});
pub static PRINTK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"printk\s*\(\s*(?:KERN_\w+\s+)?"([^"]*)"(?:,([^)]*))?\s*\)\s*;"#).unwrap()
});

pub const ASM_TO_UAGL: &[(&str, &str)] = &[
    ("dsb sy", "□ ◇    /* DSB SY */"),
    ("dsb ish", "□ ◇    /* DSB ISH */"),
    ("dsb ishst", "□ ◇    /* DSB ISHST */"),
    ("dmb sy", "□ ◇    /* DMB SY */"),
    ("dmb ish", "□ ◇    /* DMB ISH */"),
    ("isb", "◇      /* ISB */"),
    ("wfe", "□ ◇    /* WFE */"),
    ("wfi", "□ ◇    /* WFI */"),
    ("sev", "→ ◇    /* SEV */"),
    ("nop", "∅ ◇    /* NOP */"),
    ("mfence", "□ ◇    /* MFENCE */"),
    ("sfence", "□ ◇    /* SFENCE */"),
    ("lfence", "□ ◇    /* LFENCE */"),
    ("dc civac", "○ addr ◇   /* DC CIVAC */"),
    ("dc cvac", "○ addr ◇   /* DC CVAC */"),
    ("ic iallu", "○ 0 ◇      /* IC IALLU */"),
];

pub const LINUX_GVECTORS: &[(&str, GVector)] = &[
    ("sched", (0.9, 0.3, 0.9, 0.7)),
    ("irq", (0.9, 0.1, 0.6, 0.8)),
    ("mm", (0.7, 0.2, 0.8, 0.5)),
    ("fs", (0.6, 0.4, 0.7, 0.4)),
    ("net", (0.7, 0.3, 0.8, 0.6)),
    ("drivers", (0.5, 0.3, 0.6, 0.3)),
    ("arch", (0.8, 0.2, 0.7, 0.6)),
    ("kernel", (0.8, 0.3, 0.8, 0.6)),
    ("block", (0.6, 0.2, 0.7, 0.4)),
    ("crypto", (0.7, 0.4, 0.7, 0.5)),
    ("security", (0.8, 0.5, 0.7, 0.6)),
    ("default", (0.6, 0.3, 0.6, 0.4)),
];

pub fn get_linux_gvector(filepath: &str) -> GVector {
    let lower = filepath.to_lowercase();
    LINUX_GVECTORS
        .iter()
        .find(|(key, _)| lower.starts_with(key) || lower.contains(&format!("/{key}/")))
        .map_or((0.6, 0.3, 0.6, 0.4), |(_, vector)| *vector)
}

static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static MRS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^mrs\s+%0\s*,\s*(\w+)").unwrap());
static MSR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^msr\s+(\w+)\s*,\s*%0").unwrap());
static MRS_VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""=r"\s*\((\w+)\)"#).unwrap());
static MSR_VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""r"\s*\((\w+)\)"#).unwrap());
static EMPTY_LINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

pub fn translate_asm_volatile(m: &Captures<'_>, indent: &str) -> String {
    let instruction = m[1]
        .trim()
        .to_lowercase()
        .replace("\\n", " ")
        .replace("\\t", " ");
    let spaced = SPACE.replace_all(&instruction, " ");
    let instruction = spaced.trim();
    let header = format!("{indent}/* ASM: {instruction} */");
    if let Some((_, uagl)) = ASM_TO_UAGL
        .iter()
        .find(|(key, _)| instruction.starts_with(key))
    {
        return format!("{header}\n{indent}{uagl}");
    }

    let outputs = m.get(2).map_or("", |value| value.as_str().trim());
    let inputs = m.get(3).map_or("", |value| value.as_str().trim());
    if let Some(register) = MRS.captures(instruction) {
        let reg = &register[1];
        let found = MRS_VAR.captures(outputs);
        let varname = found.as_ref().map_or("result", |found| &found[1]);
        return format!(
            "{header}\n{indent}○ {varname} ─ {} ◇   /* MRS {reg} */",
            reg.to_uppercase()
        );
    }
    if let Some(register) = MSR.captures(instruction) {
        let reg = &register[1];
        let found = MSR_VAR.captures(inputs);
        let varname = found.as_ref().map_or("value", |found| &found[1]);
        return format!(
            "{header}\n{indent}○ {} ─ {varname} ◇   /* MSR {reg} */",
            reg.to_uppercase()
        );
    }
    format!("{header}\n{indent}○ /* inline asm */ ◇")
}

pub fn translate_spinlock(name: &str, lock: bool, flags: &str) -> String {
    match (lock, flags.is_empty()) {
        (true, true) => format!("□ {name} ◇   /* spin_lock */"),
        (true, false) => format!("□ {name} ─ {flags} ◇   /* spin_lock_irqsave */"),
        (false, true) => format!("→ {name} ◇   /* spin_unlock */"),
        (false, false) => format!("→ {name} ─ {flags} ◇   /* spin_unlock_irqrestore */"),
    }
}

pub fn translate_rcu(op: &str, arg: &str) -> String {
    match op {
        "read_lock" => "□ RCU ◇         /* rcu_read_lock */".to_owned(),
        "read_unlock" => "→ RCU ◇         /* rcu_read_unlock */".to_owned(),
        "deref" => format!("○ {arg} ◇       /* rcu_dereference */"),
        _ => format!("/* RCU {op} */"),
    }
}

pub fn translate_list_for_each(m: &Captures<'_>) -> String {
    let call = if m.get(2).is_some() {
        "list_for_each_entry_safe"
    } else {
        "list_for_each_entry"
    };
    format!("ⁿ {} ─ {} ─ △ {} ═╗   /* {call} */", &m[1], &m[3], &m[4])
}

pub fn translate_container_of(m: &Captures<'_>) -> String {
    format!("◈ {} ─ {} ─ △ {}   /* container_of */", &m[1], &m[2], &m[3])
}

pub fn translate_kmalloc(m: &Captures<'_>) -> String {
    let name = &m[1];
    format!("○ sizeof_{name} ◇   /* kmalloc(sizeof *{name}) */")
}

pub fn translate_per_cpu_define(m: &Captures<'_>) -> String {
    format!(
        "· {} ─ {}[8] ─ 0 ◇   /* DEFINE_PER_CPU */",
        m[2].trim(),
        map_type(m[1].trim())
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxPreprocessResult {
    pub src: String,
    pub module_init: Option<String>,
    pub module_exit: Option<String>,
    pub warnings: Vec<String>,
}

pub fn preprocess_linux(src: &str) -> LinuxPreprocessResult {
    let module_init = MODULE_INIT_RE.captures(src).map(|m| m[1].to_owned());
    let module_exit = MODULE_EXIT_RE.captures(src).map(|m| m[1].to_owned());

    let src = LINUX_STRIP_ATTRS.replace_all(src, "");
    let src = LINUX_LIKELY_RE.replace_all(&src, "$2");
    let src = LINUX_BARRIER_RE.replace_all(&src, "/* BARRIER: $1 */");
    let src = LINUX_ISB_RE.replace_all(&src, "/* ISB */");
    let src = LINUX_DSB_RE.replace_all(&src, "/* DSB $1 */");
    let src = DEFINE_SPINLOCK_RE.replace_all(&src, "unsigned long $1 = 0; /* spinlock */");
    let src = LIST_HEAD_RE.replace_all(&src, "struct list_head $1 = {&$1, &$1}");
    let src = FOR_EACH_CPU_RE.replace_all(&src, "for (int $1 = 0; $1 < 8; $1++)");
    let src = PER_CPU_DEFINE_RE.replace_all(&src, "unsigned long $2[8]; /* per_cpu */");
    let src = PER_CPU_ACCESS_RE.replace_all(&src, "$1[$2]");
    let src = PRINTK_RE.replace_all(&src, |m: &Captures<'_>| {
        format!("/* printk: {} */", m[1].trim())
    });
    let src = KMALLOC_RE.replace_all(&src, "((struct $1*)alloc(sizeof(struct $1)))");
    let src = KZALLOC_RE.replace_all(&src, "((struct $1*)calloc(sizeof(struct $1)))");
    let src = KFREE_RE.replace_all(&src, "free($1);");
    let src = SPIN_LOCK_RE.replace_all(&src, "/* LOCK: $1 */ (void)0;");
    let src = SPIN_UNLOCK_RE.replace_all(&src, "/* UNLOCK: $1 */ (void)0;");
    let src = RCU_READ_LOCK_RE.replace_all(&src, "/* RCU_READ_LOCK */ (void)0;");
    let src = RCU_READ_UNLOCK_RE.replace_all(&src, "/* RCU_READ_UNLOCK */ (void)0;");
    let src = RCU_DEREF_RE.replace_all(&src, "$1");
    let src = INIT_LIST_HEAD_RE.replace_all(&src, "/* INIT_LIST_HEAD($1) */");
    let src = LIST_ADD_RE.replace_all(&src, "/* list_add($1, $3) */");
    let src = LIST_DEL_RE.replace_all(&src, "/* list_del($1) */");
    let src = CONTAINER_OF_RE.replace_all(
        &src,
        "((struct $2 *)((char *)($1) - offsetof(struct $2, $3)))",
    );
    let src = ASM_VOLATILE_RE.replace_all(&src, |m: &Captures<'_>| {
        let text = &m[1];
        let end = text
            .char_indices()
            .nth(40)
            .map_or(text.len(), |(index, _)| index);
        format!("/* asm_volatile: {} */", text[..end].trim())
    });
    let src = EMPTY_LINES.replace_all(&src, "\n\n").into_owned();

    LinuxPreprocessResult {
        src,
        module_init,
        module_exit,
        warnings: Vec::new(),
    }
}
