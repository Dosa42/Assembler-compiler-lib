//! Main code generator — a faithful port of the `AArch64Generator` class:
//! `gen`, `_dispatch`, `_emit_file_header`, `_emit_file_footer`,
//! `_global_emit_shape`, `_emit_global`, `_begin_function`, `_end_block`,
//! `_emit_local_decl`, `_emit_zero_block`, `_emit_assign`, `_emit_return`,
//! `_emit_tuple_recv`, `_emit_simple_cmp`, `_split_top_level`, `_emit_if`,
//! `_emit_else`, `_emit_loop`, plus the two unused-but-ported helper
//! methods `_struct_field_type` / `_array_field_stride`.

use crate::call::{emit_call, emit_scope};
use crate::expr::{emit_load_value, emit_struct_base_addr};
use crate::func_state::FuncState;
use crate::helpers::{
    aarch64_addsub_imm_lines, decode_uagl_string, parse_condition, parse_params, strip_outer_parens,
    try_parse_base0,
};
use crate::tables::{GLOBAL_TABLE, STRUCT_TABLE};
use crate::types::{compute_struct_layout, dict_set, elem_log2, field_offset, field_type};
use crate::token::{Token, TK};
use once_cell::sync::Lazy;
use regex::Regex;

fn re(pat: &str) -> Regex {
    Regex::new(pat).unwrap_or_else(|e| panic!("bad regex {pat:?}: {e}"))
}

#[derive(Debug, Clone)]
pub struct GlobalDecl {
    pub name: String,
    pub r#type: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StructBuilder {
    pub name: String,
    pub fields: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub enum BlockCtx {
    Function {
        name: String,
    },
    If {
        else_label: String,
        end_label: String,
        else_emitted: bool,
    },
    Else {
        end_label: String,
    },
    Loop {
        top_label: String,
        end_label: String,
    },
}

pub struct AArch64Generator {
    pub source_name: String,
    pub out: Vec<String>,
    pub func: Option<FuncState>,
    pub in_struct: bool,
    pub in_module: bool,
    pub block_stack: Vec<BlockCtx>,
    pub globals: Vec<GlobalDecl>,
    pub structs: Vec<StructBuilder>,
    rodata_strings: Vec<(String, Vec<u8>, String)>,
    pending_if_end: Option<String>,
    #[allow(dead_code)]
    spill_idx: usize,
}

impl AArch64Generator {
    pub fn new(source_name: String) -> Self {
        AArch64Generator {
            source_name,
            out: Vec::new(),
            func: None,
            in_struct: false,
            in_module: false,
            block_stack: Vec::new(),
            globals: Vec::new(),
            structs: Vec::new(),
            rodata_strings: Vec::new(),
            pending_if_end: None,
            spill_idx: 0,
        }
    }

    #[allow(dead_code)]
    fn struct_field_type(&self, struct_name: &str, field_name: &str) -> Option<String> {
        let sname = struct_name.trim_start_matches('@');
        for s in &self.structs {
            if s.name == sname {
                for (fname, ftype) in &s.fields {
                    if fname == field_name {
                        return Some(ftype.clone());
                    }
                }
            }
        }
        None
    }

    #[allow(dead_code)]
    fn array_field_stride(&self, struct_name: &str, field_name: &str, default_elem: &str) -> (String, i64) {
        let ftype = self.struct_field_type(struct_name, field_name);
        let elem = match &ftype {
            Some(t) if t.contains('[') => t[..t.find('[').unwrap()].to_string(),
            _ => default_elem.to_string(),
        };
        let log2 = match elem.as_str() {
            "UINT8" | "U8" | "INT8" | "S8" | "BOOL" | "CHAR" => 0,
            "UINT16" | "U16" | "INT16" | "S16" => 1,
            "UINT32" | "U32" | "INT32" | "S32" | "FLOAT" => 2,
            _ => 3,
        };
        (elem, log2)
    }

    fn emit(&mut self, line: impl Into<String>) {
        self.out.push(line.into());
    }

    pub fn gen(&mut self, tokens: &[Token]) -> String {
        self.gen_tokens(tokens, false);
        self.out.join("\n")
    }

    pub(crate) fn validate(&mut self, tokens: &[Token]) {
        self.gen_tokens(tokens, true);
    }

    fn gen_tokens(&mut self, tokens: &[Token], validate: bool) {
        self.emit_file_header();

        let mut cur_struct: Option<String> = None;
        for tok in tokens {
            match tok.kind {
                TK::StructDef => {
                    let name = tok.data.name.clone().unwrap_or_default();
                    STRUCT_TABLE.lock().unwrap().entry(name.clone()).or_default();
                    cur_struct = Some(name);
                }
                TK::Field => {
                    if let Some(cs) = &cur_struct {
                        let fname = tok.data.name.clone().unwrap_or_default();
                        let ftype = tok.data.r#type.clone().unwrap_or_default();
                        let mut table = STRUCT_TABLE.lock().unwrap();
                        let entry = table.entry(cs.clone()).or_default();
                        dict_set(entry, fname, ftype);
                    }
                }
                TK::BlockClose => {
                    if cur_struct.is_some() {
                        cur_struct = None;
                    }
                }
                _ => {}
            }
        }

        for tok in tokens {
            if validate {
                crate::diagnostics::set_line(tok.line_no);
                if tok.kind == TK::Unknown {
                    crate::diagnostics::record(|| format!("Unrecognized UAGL line: {}", tok.raw.trim()));
                }
            }
            if self.pending_if_end.is_some() && tok.kind != TK::Else && tok.kind != TK::Comment {
                let label = self.pending_if_end.take().unwrap();
                self.emit(format!("{}:", label));
            }
            self.dispatch(tok);
        }
        if let Some(label) = self.pending_if_end.take() {
            self.emit(format!("{}:", label));
        }

        self.emit_file_footer();
    }

    fn dispatch(&mut self, tok: &Token) {
        match tok.kind {
            TK::Pragma => {
                let value = tok.data.value.as_deref().unwrap_or("");
                self.emit(format!("\t// #PRAGMA {}", value));
            }
            TK::Comment => {
                let text = tok.data.text.as_deref().unwrap_or("");
                let text = text.trim_matches(|c: char| c == '/' || c == '*' || c == ' ');
                self.emit(format!("\t// {}", text));
            }
            TK::ModuleOpen => {
                self.in_module = true;
                let name = tok.data.name.as_deref().unwrap_or("");
                self.emit(format!("\n\t// MODULE: {}", name));
            }
            TK::StructDef => {
                self.in_struct = true;
                let name = tok.data.name.clone().unwrap_or_default();
                self.structs.push(StructBuilder {
                    name: name.clone(),
                    fields: Vec::new(),
                });
                self.emit(format!("\t// STRUCT {}", name));
            }
            TK::Field => {
                if self.in_struct {
                    if let Some(last) = self.structs.last_mut() {
                        let fname = tok.data.name.clone().unwrap_or_default();
                        let ftype = tok.data.r#type.clone().unwrap_or_default();
                        dict_set(&mut last.fields, fname, ftype);
                    }
                }
            }
            TK::Global => {
                let data = GlobalDecl {
                    name: tok.data.name.clone().unwrap_or_default(),
                    r#type: tok.data.r#type.clone().unwrap_or_default(),
                    value: tok.data.value.clone(),
                };
                self.emit_global(data);
            }
            TK::FuncDef => self.begin_function(tok),
            TK::BlockClose => self.end_block(),
            TK::VarDecl => {
                if self.func.is_some() {
                    let name = tok.data.name.clone().unwrap_or_default();
                    let utype = tok.data.r#type.clone().unwrap_or_default();
                    let value = tok.data.value.clone().unwrap_or_else(|| "0".to_string());
                    self.emit_local_decl(&name, &utype, &value);
                }
            }
            TK::Assign => {
                if self.func.is_some() {
                    let target = tok.data.target.clone().unwrap_or_default();
                    let value = tok.data.value.clone().unwrap_or_default();
                    self.emit_assign(&target, &value);
                }
            }
            TK::Return => {
                if self.func.is_some() {
                    let value = tok.data.value.clone().unwrap_or_else(|| "0".to_string());
                    self.emit_return(&value);
                }
            }
            TK::TupleRecv => {
                if self.func.is_some() {
                    let var1 = tok.data.var1.clone().unwrap_or_default();
                    let var2 = tok.data.var2.clone().unwrap_or_default();
                    let func = tok.data.func.clone().unwrap_or_default();
                    let args = tok.data.args.clone().unwrap_or_default();
                    self.emit_tuple_recv(&var1, &var2, &func, &args);
                }
            }
            TK::If => {
                if self.func.is_some() {
                    let cond = tok.data.cond.clone().unwrap_or_default();
                    self.emit_if(&cond);
                }
            }
            TK::Else => {
                if self.func.is_some() {
                    self.emit_else();
                }
            }
            TK::Loop => {
                if self.func.is_some() {
                    let cond = tok.data.cond.clone().unwrap_or_default();
                    self.emit_loop(&cond);
                }
            }
            TK::Call => {
                if self.func.is_some() {
                    let func = tok.data.func.clone().unwrap_or_default();
                    let args = tok.data.args.clone().unwrap_or_default();
                    emit_call(&func, &args, self.func.as_ref(), &mut self.out);
                }
            }
            TK::Barrier => {
                self.emit("\tDSB\tSY");
                self.emit("\tISB");
            }
            TK::Scope => {
                if self.func.is_some() {
                    let args = tok.data.args.clone().unwrap_or_default();
                    emit_scope(&args, self.func.as_ref(), &mut self.out);
                }
            }
            TK::Validate => {
                if self.func.is_some() {
                    let args = tok.data.args.clone().unwrap_or_default();
                    static SEP_RE: Lazy<Regex> = Lazy::new(|| re(r"\s*─\s*"));
                    let arg_list: Vec<String> = SEP_RE
                        .split(&args)
                        .map(|a| a.trim().to_string())
                        .filter(|a| !a.is_empty())
                        .collect();
                    for (i, a) in arg_list.iter().take(4).enumerate() {
                        emit_load_value(a, &format!("X{}", i), self.func.as_ref(), &mut self.out, "UINT64");
                    }
                    self.emit("\tSMC\t#0");
                }
            }
            TK::Nop => {
                self.emit("\tNOP");
            }
            _ => {}
        }
    }

    // ── File header ─────────────────────────────────────────────────────

    fn emit_file_header(&mut self) {
        self.emit("// Generated by UAGL v3 → AArch64 Assembler");
        self.emit(format!("// Source: {}", self.source_name));
        self.emit("// AArch64 GNU assembly (AArch64 ELF)");
        self.emit("");
        self.emit("\t.arch\tarmv8-a");
        self.emit("\t.text");
        self.emit("");
    }

    fn emit_file_footer(&mut self) {
        if !self.globals.is_empty() {
            self.emit("\n\t.section\t.data");
            let mut current_section = ".data".to_string();
            let globals = self.globals.clone();
            for g in &globals {
                let name = &g.name;
                let utype = &g.r#type;
                let val = g
                    .value
                    .clone()
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| "0".to_string());
                let val = val.trim().to_string();

                let mut want_section = ".data".to_string();
                if utype.contains('[')
                    && ((val.starts_with('[') && val.ends_with(']'))
                        || (val.starts_with('{') && val.ends_with('}'))
                        || (val.starts_with('"') && val.ends_with('"')))
                {
                    want_section = ".rodata".to_string();
                } else if !utype.contains('[') {
                    if let Some(n) = try_parse_base0(&val) {
                        if n != 0 {
                            want_section = ".rodata".to_string();
                        }
                    }
                }
                if want_section != current_section {
                    self.emit(format!("\t.section\t{}", want_section));
                    current_section = want_section;
                }

                let is_array = utype.contains('[');
                let (mut elem_sz, mut directive, mut align_log2);
                let elem_t: String;
                if is_array {
                    elem_t = utype[..utype.find('[').unwrap()].trim().to_string();
                    let shape = Self::global_emit_shape(&elem_t);
                    elem_sz = shape.0;
                    directive = shape.1;
                    align_log2 = shape.2;
                } else {
                    elem_t = utype.trim().to_string();
                    let shape = Self::global_emit_shape(&elem_t);
                    elem_sz = shape.0;
                    directive = shape.1;
                    align_log2 = 3;
                }

                if STRUCT_TABLE.lock().unwrap().contains_key(&elem_t) {
                    let (_, struct_sz) = compute_struct_layout(&elem_t);
                    elem_sz = struct_sz;
                    align_log2 = 3;
                    directive = ".skip";
                    let _ = directive;
                }

                self.emit(format!("\t.global\t{}", name));
                self.emit(format!("\t.align\t{}", align_log2));
                self.emit(format!("{}:", name));

                if is_array
                    && matches!(elem_t.as_str(), "U8" | "UINT8" | "CHAR" | "INT8" | "I8" | "BOOL")
                    && val.starts_with('"')
                    && val.ends_with('"')
                    && val.chars().count() >= 2
                {
                    let inner = crate::helpers::drop_first_last_char(&val);
                    let mut decoded = decode_uagl_string(&inner);
                    let mut declared: i64 = 1;
                    static COUNT_RE: Lazy<Regex> = Lazy::new(|| re(r"\[(\d+)\]"));
                    if let Some(caps) = COUNT_RE.captures(utype) {
                        declared = caps[1].parse().unwrap_or(1);
                    }
                    if !decoded.contains(&0u8) && (decoded.len() as i64) < declared {
                        decoded.push(0);
                    }
                    if (decoded.len() as i64) > declared {
                        decoded.truncate(declared.max(0) as usize);
                    }
                    let bytes_str = decoded
                        .iter()
                        .map(|b| format!("0x{:02x}", b))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.emit(format!("\t.byte\t{}\t// {}", bytes_str, val));
                    if (decoded.len() as i64) < declared {
                        self.emit(format!("\t.zero\t{}", declared - decoded.len() as i64));
                    }
                    continue;
                }

                if is_array
                    && ((val.starts_with('[') && val.ends_with(']'))
                        || (val.starts_with('{') && val.ends_with('}')))
                {
                    let inner = crate::helpers::drop_first_last_char(&val);
                    let elems: Vec<String> = inner
                        .split(',')
                        .map(|e| e.trim().to_string())
                        .filter(|e| !e.is_empty())
                        .collect();
                    let mut declared: i64 = 1;
                    static COUNT_RE: Lazy<Regex> = Lazy::new(|| re(r"\[(\d+)\]"));
                    if let Some(caps) = COUNT_RE.captures(utype) {
                        declared = caps[1].parse().unwrap_or(1);
                    }
                    let take_n = declared.max(0) as usize;
                    for e in elems.iter().take(take_n) {
                        match try_parse_base0(e) {
                            Some(n) => self.emit(format!("\t{}\t{}", directive, n)),
                            None => self.emit(format!("\t{}\t0  // unparsed: {}", directive, e)),
                        }
                    }
                    let taken = elems.len().min(take_n) as i64;
                    if declared > taken {
                        self.emit(format!("\t.zero\t{}", (declared - taken) * elem_sz));
                    }
                    continue;
                }

                if !is_array && STRUCT_TABLE.lock().unwrap().contains_key(&elem_t) {
                    self.emit(format!("\t.zero\t{}\t// struct {} ({}B)", elem_sz, elem_t, elem_sz));
                    continue;
                }

                if is_array {
                    static COUNT_RE: Lazy<Regex> = Lazy::new(|| re(r"\[(\d+)\]"));
                    let count: i64 = COUNT_RE
                        .captures(utype)
                        .and_then(|c| c[1].parse().ok())
                        .unwrap_or(1);
                    let n = try_parse_base0(&val).unwrap_or(0);
                    if n == 0 {
                        self.emit(format!("\t.zero\t{}\t// {}", count * elem_sz, utype));
                    } else {
                        self.emit(format!("\t{}\t{}\t// {}[0]", directive, n, utype));
                        if count > 1 {
                            self.emit(format!("\t.zero\t{}", (count - 1) * elem_sz));
                        }
                    }
                    continue;
                }

                match try_parse_base0(&val) {
                    Some(n) => self.emit(format!("\t.quad\t{}\t// {}", n, utype)),
                    None => {
                        if let Some(sym) = val.strip_prefix('&') {
                            let sym = sym.trim();
                            static SYM_RE: Lazy<Regex> = Lazy::new(|| re(r"^[A-Za-z_]\w*$"));
                            if SYM_RE.is_match(sym) {
                                self.emit(format!("\t.quad\t{}\t// {} = &{}", sym, utype, sym));
                                continue;
                            }
                        }
                        self.emit(format!("\t.quad\t0  // {}", val));
                    }
                }
            }
        }

        if !self.rodata_strings.is_empty() {
            self.emit("\n\t.section\t.rodata");
            let entries = self.rodata_strings.clone();
            for (label, payload, original) in &entries {
                self.emit("\t.align\t0");
                self.emit(format!("{}:", label));
                let bytes_str = payload
                    .iter()
                    .map(|b| format!("0x{:02x}", b))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.emit(format!("\t.byte\t{}\t// {}", bytes_str, original));
            }
        }

        self.emit("\n\t.end");
        self.emit("");
    }

    fn global_emit_shape(elem_t: &str) -> (i64, &'static str, i64) {
        match elem_t {
            "UINT8" | "U8" | "INT8" | "S8" | "BOOL" | "CHAR" => (1, ".byte", 0),
            "UINT16" | "U16" | "INT16" | "S16" => (2, ".short", 1),
            "UINT32" | "U32" | "INT32" | "S32" | "FLOAT" => (4, ".word", 2),
            _ => (8, ".quad", 3),
        }
    }

    // ── Global variable ─────────────────────────────────────────────────

    fn emit_global(&mut self, data: GlobalDecl) {
        let name = data.name.clone();
        let typ = data.r#type.trim().to_string();
        static LEAD_TYPE_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*\["));
        let elem = LEAD_TYPE_RE
            .captures(&typ)
            .map(|c| c[1].to_string())
            .unwrap_or_else(|| typ.clone());
        GLOBAL_TABLE.lock().unwrap().insert(name.clone(), elem);
        self.emit(format!("\t// global: {} : {}", name, typ));
        self.globals.push(data);
    }

    // ── Function ─────────────────────────────────────────────────────────

    fn begin_function(&mut self, tok: &Token) {
        let name = tok.data.name.clone().unwrap_or_default();
        let params_raw = tok.data.params_raw.clone().unwrap_or_default();
        let (params, ret_type) = parse_params(&params_raw);

        self.func = Some(FuncState::new(name.clone(), params.clone(), ret_type.clone()));
        self.block_stack.clear();

        let is_header = self.source_name.ends_with(".h") || self.source_name.contains(".h.");
        if is_header {
            self.emit(format!("\n\t.weak\t{}  // header decl", name));
        } else {
            self.emit(format!("\n\t.global\t{}", name));
        }
        self.emit(format!("\t.type\t{}, %function", name));
        self.emit(format!("{}:", name));
        self.emit(format!("\t// params: {}  ret: {}", py_repr_params(&params), ret_type));

        self.emit("\tSTP\tX29, X30, [SP, #-16]!");
        self.func.as_mut().unwrap().stack_alloc_idx = self.out.len() as i64;
        self.emit("");
        self.emit("\tMOV\tX29, SP");

        self.spill_idx = self.out.len();
        for (i, (pname, ptype)) in params.iter().take(8).enumerate() {
            let off = self.func.as_mut().unwrap().alloc_local(pname, ptype);
            self.emit(format!("\tSTR\tX{}, [X29, #{}]\t// spill param {}", i, off, pname));
        }

        self.block_stack.push(BlockCtx::Function { name });
    }

    fn end_block(&mut self) {
        if self.in_struct {
            self.in_struct = false;
            return;
        }
        if self.block_stack.is_empty() {
            return;
        }
        let ctx = self.block_stack.pop().unwrap();
        match ctx {
            BlockCtx::Function { name } => {
                if let Some(func) = &self.func {
                    if func.stack_alloc_idx >= 0 && func.stack_size != 0 {
                        let aligned = (func.stack_size + 15) & !15;
                        let lines = aarch64_addsub_imm_lines(
                            "SUB",
                            "SP",
                            "SP",
                            aligned,
                            &format!("// alloc {}B locals", aligned),
                        );
                        self.out[func.stack_alloc_idx as usize] = lines.join("\n");
                    }
                }
                self.emit(format!("\n.{}_return:", name));
                if let Some(func) = &self.func {
                    if func.stack_size != 0 {
                        let aligned = (func.stack_size + 15) & !15;
                        for ln in aarch64_addsub_imm_lines("ADD", "SP", "SP", aligned, "// free locals") {
                            self.emit(ln);
                        }
                    }
                }
                self.emit("\tLDP\tX29, X30, [SP], #16");
                self.emit("\tRET");
                self.emit(format!("\t.size\t{}, . - {}", name, name));
                self.func = None;
                self.in_module = false;
            }
            BlockCtx::If {
                end_label,
                else_label,
                else_emitted,
            } => {
                if !else_emitted {
                    self.emit(format!("\tB\t{}", end_label));
                    self.emit(format!("{}:", else_label));
                    self.pending_if_end = Some(end_label);
                } else {
                    self.emit(format!("{}:", end_label));
                }
            }
            BlockCtx::Else { end_label } => {
                self.emit(format!("{}:", end_label));
            }
            BlockCtx::Loop { top_label, end_label } => {
                self.emit(format!("\tB\t{}", top_label));
                self.emit(format!("{}:", end_label));
            }
        }
    }

    // ── Local variable ──────────────────────────────────────────────────

    fn emit_local_decl(&mut self, name: &str, utype: &str, val: &str) {
        if self.func.is_none() {
            return;
        }
        if !self.func.as_ref().unwrap().locals.contains_key(name) {
            self.func.as_mut().unwrap().alloc_local(name, utype);
        }
        let off = self.func.as_ref().unwrap().offset_of(name).unwrap();
        self.emit(format!("\t// let {}: {} = {}", name, utype, val));

        static ARR_TYPE_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*\[\s*(\d+)\s*\]$"));
        let arr_caps = ARR_TYPE_RE.captures(utype.trim());

        if let Some(caps) = &arr_caps {
            if matches!(val.trim(), "0" | "{}" | "{ 0 }" | "") {
                let elem_t = caps[1].to_string();
                let n: i64 = caps[2].parse().unwrap_or(0);
                let total_bytes = n << elem_log2(&elem_t);
                self.emit_zero_block(off, total_bytes);
                return;
            }
        }

        let v = val.trim();
        if let Some(caps) = &arr_caps {
            let elem_name = &caps[1];
            if matches!(elem_name, "U8" | "UINT8" | "CHAR" | "INT8" | "I8" | "BOOL")
                && v.starts_with('"')
                && v.ends_with('"')
                && v.chars().count() >= 2
            {
                let n: i64 = caps[2].parse().unwrap_or(0);
                let inner = crate::helpers::drop_first_last_char(v);
                let mut decoded = decode_uagl_string(&inner);
                if !decoded.contains(&0u8) && (decoded.len() as i64) < n {
                    decoded.push(0);
                }
                if (decoded.len() as i64) > n {
                    decoded.truncate(n.max(0) as usize);
                }
                let label = self.func.as_ref().unwrap().next_label("strlit");
                self.rodata_strings.push((label.clone(), decoded.clone(), v.to_string()));
                self.emit(format!("\tADRP\tX10, {}", label));
                self.emit(format!("\tADD\tX10, X10, :lo12:{}", label));
                for ln in aarch64_addsub_imm_lines("ADD", "X11", "X29", off, "// &local = stack base") {
                    self.emit(ln);
                }
                self.emit(format!("\tMOV\tX12, #{}", decoded.len()));
                let tag = self.func.as_ref().unwrap().next_label("strlit_cp");
                self.emit(format!("{}:", tag));
                self.emit(format!("\tCBZ\tX12, {}_end", tag));
                self.emit("\tLDRB\tW9, [X10], #1");
                self.emit("\tSTRB\tW9, [X11], #1");
                self.emit("\tSUB\tX12, X12, #1");
                self.emit(format!("\tB\t{}", tag));
                self.emit(format!("{}_end:", tag));
                let tail = n - decoded.len() as i64;
                if tail > 0 {
                    self.emit_zero_block(off + decoded.len() as i64, tail);
                }
                return;
            }
        }

        let u = utype.trim();
        if STRUCT_TABLE.lock().unwrap().contains_key(u) && matches!(val.trim(), "0" | "{}" | "{ 0 }" | "") {
            let (_, total_bytes) = compute_struct_layout(u);
            self.emit_zero_block(off, total_bytes);
            return;
        }

        emit_load_value(val, "X9", self.func.as_ref(), &mut self.out, utype);
        self.emit(format!("\tSTR\tX9, [X29, #{}]", off));
    }

    fn emit_zero_block(&mut self, off: i64, total_bytes: i64) {
        if total_bytes <= 0 {
            return;
        }
        if total_bytes <= 256 {
            let mut pos = 0i64;
            while pos + 16 <= total_bytes {
                let cur = off + pos;
                if (-512..=504).contains(&cur) && cur % 8 == 0 {
                    self.emit(format!("\tSTP\tXZR, XZR, [X29, #{}]", cur));
                } else {
                    self.emit(format!("\tSTR\tWZR, [X29, #{}]", cur));
                    self.emit(format!("\tSTR\tWZR, [X29, #{}]", cur + 4));
                    self.emit(format!("\tSTR\tWZR, [X29, #{}]", cur + 8));
                    self.emit(format!("\tSTR\tWZR, [X29, #{}]", cur + 12));
                }
                pos += 16;
            }
            while pos + 8 <= total_bytes {
                let cur = off + pos;
                if (0..=32760).contains(&cur) && cur % 8 == 0 {
                    self.emit(format!("\tSTR\tXZR, [X29, #{}]", cur));
                } else {
                    self.emit(format!("\tSTR\tWZR, [X29, #{}]", cur));
                    self.emit(format!("\tSTR\tWZR, [X29, #{}]", cur + 4));
                }
                pos += 8;
            }
            while pos + 4 <= total_bytes {
                let cur = off + pos;
                self.emit(format!("\tSTR\tWZR, [X29, #{}]", cur));
                pos += 4;
            }
            while pos < total_bytes {
                let cur = off + pos;
                self.emit(format!("\tSTRB\tWZR, [X29, #{}]", cur));
                pos += 1;
            }
            return;
        }

        for ln in aarch64_addsub_imm_lines("ADD", "X10", "X29", off, "// zero-init cursor") {
            self.emit(ln);
        }
        for ln in aarch64_addsub_imm_lines("ADD", "X11", "X10", total_bytes, "// zero-init end") {
            self.emit(ln);
        }
        let top = self.func.as_ref().unwrap().next_label("zinit_top");
        let end = self.func.as_ref().unwrap().next_label("zinit_end");
        self.emit(format!("{}:", top));
        self.emit("\tCMP\tX10, X11");
        self.emit(format!("\tB.HS\t{}", end));
        self.emit("\tSTP\tXZR, XZR, [X10], #16");
        self.emit(format!("\tB\t{}", top));
        self.emit(format!("{}:", end));
    }

    // ── Assignment ───────────────────────────────────────────────────────

    fn emit_assign(&mut self, target: &str, val: &str) {
        let target = target.trim().to_string();
        let mut val = val.trim().to_string();

        if val == "─" {
            static SIMPLE_VAR_RE: Lazy<Regex> = Lazy::new(|| re(r"^\w+$"));
            static ARRAY_ELEM_RE: Lazy<Regex> = Lazy::new(|| re(r"^\w+\[[^\]]+\]$"));
            static STRUCT_FIELD_RE: Lazy<Regex> =
                Lazy::new(|| re(r"^\w+\s*─\s*△\s*\w+(\[[^\]]+\])?$"));
            if SIMPLE_VAR_RE.is_match(&target)
                || ARRAY_ELEM_RE.is_match(&target)
                || STRUCT_FIELD_RE.is_match(&target)
            {
                val = format!("{} + 1", target);
            }
        }

        self.emit(format!("\t// {} = {}", target, val));
        emit_load_value(&val, "X9", self.func.as_ref(), &mut self.out, "UINT64");

        static FIELD_SUB_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*─\s*△\s*(\w+)\s*\[([^\]]+)\]$"));
        if let Some(caps) = FIELD_SUB_RE.captures(&target) {
            let obj = caps[1].to_string();
            let field = caps[2].to_string();
            let idx = caps[3].trim().to_string();
            let obj_t = self
                .func
                .as_ref()
                .map(|f| f.type_of(&obj))
                .unwrap_or_else(|| "@".to_string());
            let (elem, log2) = crate::types::struct_field_elem_stride(&obj_t, &field, "INT32");
            let f_off = field_offset(&obj_t, &field);
            self.emit("\tSTR\tX9, [SP, #-16]!");
            emit_struct_base_addr(&obj, &obj_t, "X11", self.func.as_ref(), &mut self.out);
            if f_off != 0 {
                for ln in aarch64_addsub_imm_lines("ADD", "X11", "X11", f_off, &format!("// .{} offset", field)) {
                    self.emit(ln);
                }
            }
            emit_load_value(&idx, "X10", self.func.as_ref(), &mut self.out, "UINT64");
            self.emit("\tLDR\tX9, [SP], #16");
            match elem.as_str() {
                "UINT8" | "U8" | "INT8" | "S8" | "BOOL" | "CHAR" => {
                    self.emit(format!("\tSTRB\tW9, [X11, X10]\t// .{}[idx] byte", field));
                }
                "UINT32" | "U32" | "INT32" | "S32" | "FLOAT" => {
                    self.emit(format!("\tSTR\tW9, [X11, X10, LSL #{}]\t// .{}[idx] 32b", log2, field));
                }
                _ => {
                    self.emit(format!("\tSTR\tX9, [X11, X10, LSL #{}]\t// .{}[idx] {}", log2, field, elem));
                }
            }
            return;
        }

        static FIELD_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\s*─\s*△\s*(\w+)$"));
        if let Some(caps) = FIELD_RE.captures(&target) {
            let obj_name = caps[1].to_string();
            let field_name = caps[2].to_string();
            let obj_t = self
                .func
                .as_ref()
                .map(|f| f.type_of(&obj_name))
                .unwrap_or_else(|| "@".to_string());
            let f_off = field_offset(&obj_t, &field_name);
            let ftype = field_type(&obj_t, &field_name);
            emit_struct_base_addr(&obj_name, &obj_t, "X10", self.func.as_ref(), &mut self.out);
            if f_off != 0 {
                for ln in aarch64_addsub_imm_lines(
                    "ADD",
                    "X10",
                    "X10",
                    f_off,
                    &format!("// .{} offset", field_name),
                ) {
                    self.emit(ln);
                }
            }
            let mut inner = ftype.trim_start_matches('@').trim().to_string();
            if let Some(i) = inner.find('[') {
                inner = inner[..i].to_string();
            }
            match inner.as_str() {
                "UINT8" | "U8" | "INT8" | "S8" | "BOOL" | "CHAR" => {
                    self.emit(format!("\tSTRB\tW9, [X10]\t// .{} byte", field_name));
                }
                "UINT32" | "U32" | "INT32" | "S32" | "FLOAT" => {
                    self.emit(format!("\tSTR\tW9, [X10]\t// .{} 32b", field_name));
                }
                _ => {
                    self.emit(format!("\tSTR\tX9, [X10]\t// .{}", field_name));
                }
            }
            return;
        }

        static ARR_SUB_RE: Lazy<Regex> = Lazy::new(|| re(r"^(\w+)\[([^\]]+)\]$"));
        if let Some(caps) = ARR_SUB_RE.captures(&target) {
            let arr = caps[1].to_string();
            let idx = caps[2].trim().to_string();
            if arr == "mem" {
                self.emit("\tSTR\tX9, [SP, #-16]!");
                emit_load_value(&idx, "X10", self.func.as_ref(), &mut self.out, "UINT64");
                self.emit("\tLDR\tX9, [SP], #16");
                self.emit("\tSTR\tW9, [X10]\t// mem[addr] = val (MMIO 32b)");
                return;
            }
            self.emit("\tSTR\tX9, [SP, #-16]!");
            emit_load_value(&idx, "X10", self.func.as_ref(), &mut self.out, "UINT64");
            self.emit("\tLDR\tX9, [SP], #16");
            let off = self.func.as_ref().and_then(|f| f.offset_of(&arr));
            let arr_t: String;
            if let Some(off) = off {
                let t = self.func.as_ref().unwrap().type_of(&arr);
                if t.starts_with('@') || t == "PTR" {
                    self.emit(format!("\tLDR\tX11, [X29, #{}]\t// load ptr {}", off, arr));
                } else {
                    for ln in aarch64_addsub_imm_lines("ADD", "X11", "X29", off, "") {
                        self.emit(ln);
                    }
                }
                arr_t = t;
            } else {
                self.emit(format!("\tADRP\tX11, {}", arr));
                self.emit(format!("\tADD\tX11, X11, :lo12:{}", arr));
                arr_t = crate::types::global_array_elem_stride(&arr, "INT32").0;
            }
            let elem_t = if let Some(rest) = arr_t.strip_prefix('@') {
                rest.to_string()
            } else if let Some(i) = arr_t.find('[') {
                arr_t[..i].to_string()
            } else {
                arr_t.clone()
            };
            match elem_t.as_str() {
                "UINT8" | "U8" | "INT8" | "S8" | "BOOL" | "CHAR" => {
                    self.emit(format!("\tSTRB\tW9, [X11, X10]\t// {}[idx] byte", arr));
                }
                "INT32" | "UINT32" | "S32" | "U32" => {
                    self.emit(format!("\tSTR\tW9, [X11, X10, LSL #2]\t// {}[idx] 32b", arr));
                }
                _ => {
                    self.emit(format!("\tSTR\tX9, [X11, X10, LSL #3]\t// {}[idx] 64b", arr));
                }
            }
            return;
        }

        if let Some(func) = &self.func {
            if let Some(off) = func.offset_of(&target) {
                self.emit(format!("\tSTR\tX9, [X29, #{}]", off));
                return;
            }
        }

        static SYMBOL_RE: Lazy<Regex> = Lazy::new(|| re(r"^[A-Za-z_]\w*$"));
        if SYMBOL_RE.is_match(&target) {
            self.emit(format!("\tADRP\tX11, {}", target));
            self.emit(format!("\tADD\tX11, X11, :lo12:{}", target));
            self.emit("\tSTR\tX9, [X11]");
        }
    }

    // ── Return ───────────────────────────────────────────────────────────

    fn emit_return(&mut self, val: &str) {
        if val != "◇" && !val.trim().is_empty() {
            let ret_t = self
                .func
                .as_ref()
                .map(|f| f.ret_type.clone())
                .unwrap_or_else(|| "UINT64".to_string());

            if val.contains('‖') {
                let parts: Vec<String> = val.splitn(2, '‖').map(|p| p.trim().to_string()).collect();
                if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                    emit_load_value(&parts[1], "X9", self.func.as_ref(), &mut self.out, &ret_t);
                    self.emit("\tSTR\tX9, [SP, #-16]!\t// spill v2 across v1 eval");
                    emit_load_value(&parts[0], "X0", self.func.as_ref(), &mut self.out, &ret_t);
                    self.emit("\tLDR\tX1, [SP], #16\t// reload v2 into X1");
                    let fname = self
                        .func
                        .as_ref()
                        .map(|f| f.name.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    self.emit(format!("\tB\t.{}_return", fname));
                    return;
                }
            }
            emit_load_value(val, "X0", self.func.as_ref(), &mut self.out, &ret_t);
        }
        let fname = self
            .func
            .as_ref()
            .map(|f| f.name.clone())
            .unwrap_or_else(|| "unknown".to_string());
        self.emit(format!("\tB\t.{}_return", fname));
    }

    fn emit_tuple_recv(&mut self, var1: &str, var2: &str, func: &str, args: &str) {
        let var1 = var1.trim().to_string();
        let var2 = var2.trim().to_string();
        let func = func.trim().to_string();
        self.emit(format!("\t// ← {} ─ {} ─ {} ○ {}", var1, var2, func, args));
        emit_call(&func, args, self.func.as_ref(), &mut self.out);
        for (tname, treg) in [(var2.as_str(), "X1"), (var1.as_str(), "X0")] {
            let off = self.func.as_ref().and_then(|f| f.offset_of(tname));
            if let Some(off) = off {
                self.emit(format!("\tSTR\t{}, [X29, #{}]\t// {} ← {}", treg, off, tname, treg));
            } else {
                self.emit(format!("\tADRP\tX11, {}", tname));
                self.emit(format!("\tADD\tX11, X11, :lo12:{}", tname));
                self.emit(format!("\tSTR\t{}, [X11]\t// {} ← {} (global)", treg, tname, treg));
            }
        }
    }

    // ── If/else ──────────────────────────────────────────────────────────

    fn emit_simple_cmp(&mut self, cond: &str, else_label: &str) {
        let cond = strip_outer_parens(cond);
        let (lhs, op, rhs) = parse_condition(&cond);
        emit_load_value(&lhs, "X9", self.func.as_ref(), &mut self.out, "UINT64");
        if rhs == "0" && (op == "╱" || op == "!=") {
            self.emit(format!("\tCBZ\tX9, {}", else_label));
        } else if rhs == "0" && (op == "△" || op == "==") {
            self.emit(format!("\tCBNZ\tX9, {}", else_label));
        } else {
            self.emit("\tSTR\tX9, [SP, #-16]!");
            emit_load_value(&rhs, "X10", self.func.as_ref(), &mut self.out, "UINT64");
            self.emit("\tLDR\tX9, [SP], #16");
            self.emit("\tCMP\tX9, X10");
            let branch = inverse_branch(&op);
            self.emit(format!("\t{}\t{}", branch, else_label));
        }
    }

    fn split_top_level(&self, s: &str, ch: char) -> Option<(String, String)> {
        let mut paren = 0i64;
        let mut bracket = 0i64;
        let chars: Vec<char> = s.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            match c {
                '(' => paren += 1,
                ')' => paren -= 1,
                '[' => bracket += 1,
                ']' => bracket -= 1,
                _ if paren == 0 && bracket == 0 && c == ch => {
                    let lhs: String = chars[..i].iter().collect::<String>().trim().to_string();
                    let rhs: String = chars[i + 1..].iter().collect::<String>().trim().to_string();
                    return Some((lhs, rhs));
                }
                _ => {}
            }
        }
        None
    }

    fn emit_if(&mut self, cond: &str) {
        if self.func.is_none() {
            return;
        }
        let else_label = self.func.as_ref().unwrap().next_label("else");
        let end_label = self.func.as_ref().unwrap().next_label("end");

        let cond_stripped = strip_outer_parens(cond);

        if let Some((a, b)) = self.split_top_level(&cond_stripped, '∧') {
            self.emit_simple_cmp(&a, &else_label);
            self.emit_simple_cmp(&b, &else_label);
        } else if let Some((a, b)) = self.split_top_level(&cond_stripped, '∨') {
            let body_label = self.func.as_ref().unwrap().next_label("orbody");
            let a_else = self.func.as_ref().unwrap().next_label("orelse");
            self.emit_simple_cmp(&a, &a_else);
            self.emit(format!("\tB\t{}", body_label));
            self.emit(format!("{}:", a_else));
            self.emit_simple_cmp(&b, &else_label);
            self.emit(format!("{}:", body_label));
        } else {
            self.emit_simple_cmp(&cond_stripped, &else_label);
        }

        self.block_stack.push(BlockCtx::If {
            else_label,
            end_label,
            else_emitted: false,
        });
    }

    fn emit_else(&mut self) {
        if let Some(end_label) = self.pending_if_end.take() {
            self.block_stack.push(BlockCtx::Else { end_label });
            return;
        }

        if self.block_stack.is_empty() {
            return;
        }
        let is_if = matches!(self.block_stack.last(), Some(BlockCtx::If { .. }));
        if !is_if {
            return;
        }
        let (end_label, else_label) = match self.block_stack.last().unwrap() {
            BlockCtx::If { end_label, else_label, .. } => (end_label.clone(), else_label.clone()),
            _ => unreachable!(),
        };
        self.emit(format!("\tB\t{}", end_label));
        self.emit(format!("{}:", else_label));
        let last = self.block_stack.last_mut().unwrap();
        *last = BlockCtx::Else { end_label };
    }

    // ── Loop ─────────────────────────────────────────────────────────────

    fn emit_loop(&mut self, cond: &str) {
        if self.func.is_none() {
            return;
        }
        let top_label = self.func.as_ref().unwrap().next_label("loop_top");
        let end_label = self.func.as_ref().unwrap().next_label("loop_end");

        self.emit(format!("{}:", top_label));

        let stripped = strip_outer_parens(cond);
        let (lhs, op, rhs) = parse_condition(&stripped);
        emit_load_value(&lhs, "X9", self.func.as_ref(), &mut self.out, "UINT64");

        if rhs == "0" && (op == "╱" || op == "!=") {
            self.emit(format!("\tCBZ\tX9, {}", end_label));
        } else if rhs == "0" && (op == "△" || op == "==") {
            self.emit(format!("\tCBNZ\tX9, {}", end_label));
        } else {
            self.emit("\tSTR\tX9, [SP, #-16]!");
            emit_load_value(&rhs, "X10", self.func.as_ref(), &mut self.out, "UINT64");
            self.emit("\tLDR\tX9, [SP], #16");
            self.emit("\tCMP\tX9, X10");
            let branch = inverse_branch(&op);
            self.emit(format!("\t{}\t{}", branch, end_label));
        }

        self.block_stack.push(BlockCtx::Loop { top_label, end_label });
    }
}

fn inverse_branch(op: &str) -> &'static str {
    match op {
        ">" => "B.LE",
        "<" => "B.GE",
        ">=" => "B.LT",
        "<=" => "B.GT",
        "==" => "B.NE",
        "!=" => "B.EQ",
        "△" => "B.NE",
        "╱" => "B.EQ",
        _ => "B.EQ",
    }
}

/// Render `[(name, type), ...]` the way Python's default list/tuple `repr`
/// would for the `// params: {params} ret: ...` debug comment (single
/// quotes, `, ` separators) — this only affects a comment's exact text.
fn py_repr_params(params: &[(String, String)]) -> String {
    let items: Vec<String> = params
        .iter()
        .map(|(n, t)| format!("('{}', '{}')", n, t))
        .collect();
    format!("[{}]", items.join(", "))
}
