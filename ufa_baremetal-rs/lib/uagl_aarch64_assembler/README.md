# uagl_aarch64_assembler (Rust port)

A faithful Rust port of `aarch64_assembler_braille.py` — the UAGL v3 →
AArch64 GNU-assembler backend, including the `.uaglbr` (UAGL Braille
Latin) container format.

## Layout

Mirrors the original file's logical sections, one module each:

| Module | Ports |
|---|---|
| `token.rs` | `TK`, `Token` |
| `lexer.rs` | all regexes, `_collapse_multiline_array_globals`, `lex_uagl` |
| `tables.rs` | `_STRUCT_TABLE` / `_GLOBAL_TABLE` / `_STRUCT_LAYOUTS` (process-lifetime global state) + `TYPE_TO_REG` / `SYSREG_MAP` / `CMP_TO_BRANCH` |
| `types.rs` | `type_size`, `_type_size_align`, `_compute_struct_layout`, `_field_offset`, `_field_type`, `_elem_log2`, `_struct_field_elem_stride`, `_global_array_elem_stride` |
| `func_state.rs` | `FuncState`, `reg_for` |
| `helpers.rs` | `parse_params`, `parse_condition`, `_looks_like_type`, `_strip_outer_parens`, `_has_top_level`, `_split_binop`, `_decode_uagl_string`, `_aarch64_addsub_imm_lines`, `_fp_ldr_lines`, `_fp_str_lines` |
| `expr.rs` | `_operand_aarch64_type`, `_emit_struct_base_addr`, `emit_load_value` |
| `call.rs` | `_split_call_args`, `emit_call`, `emit_comparison`, `emit_scope` |
| `codegen.rs` | `AArch64Generator` and all its methods |
| `uaglbr.rs` | the `.uaglbr` container format (encode map, canonical JSON, `decode_uaglbr_bytes`, `load_uagl_source`) |
| `lib.rs` | `assemble_uagl_str` / `assemble_uagl_bytes` (Rust's static typing splits Python's single `str \| bytes` overload in two), `assemble_file`, `assemble_batch` |

## Build & run

```
cargo build --release -p uagl_aarch64_assembler
cargo test --release -p uagl_aarch64_assembler
cargo run -p uagl_aarch64_assembler --example run_one -- file.uagl
cargo run -p uagl_aarch64_assembler --example decode_uaglbr -- file.uaglbr
```

## How this was verified

Rather than eyeballing the port, I ran the actual Python file and the
Rust port side by side on the same inputs and diffed the output:

- **8 UAGL programs** (`tests/fixtures/*.uagl`) covering: arithmetic,
  if/else, loops, functions with up to 9 args (testing the 8-arg
  register-passing truncation); structs (including pointer params,
  multi-level field chains, struct arrays); globals of every type
  (scalar, array, string, pointer, struct array); local string
  literals and zero-init (including a 300-byte local array, to
  exercise the runtime zero-init loop rather than the unrolled one);
  `__memcpy`/`__memset`/`__asm__` builtins; indirect (function-pointer)
  calls; `MRS`/`MSR` register access; tuple-returning calls; barriers;
  `VALIDATE`/SMC; unary `-`/`~`; `&`-of in all four forms; `mem[]` MMIO;
  operator precedence; hex/binary/huge/negative integer literals;
  the `x ─ ─` increment shorthand across all four lvalue shapes; `∧`/`∨`
  compound conditions; module markers; block comments; and two repro
  snippets taken verbatim from `COMPILER_BUGS.md` (Bug #4's 4-shape
  test, and the still-open Bug #66 case).
- **9 `.uaglbr` containers**, one valid and eight corrupted in
  different ways (bad magic, tampered payload, non-canonical JSON,
  truncated metadata, tampered bitmap, extra schema key, wrong
  version, negative field), built by hand from the Python file's own
  primitives.
- `assemble_file`'s file-I/O wrapper, end to end.

All of the above produced **byte-for-byte identical** output (and, for
the container tests, identical accept/reject decisions with identical
error text) between the two implementations. `tests/regression.rs`
freezes the 8 program cases as snapshots so `cargo test` keeps
checking this without needing Python installed. That test suite runs
as a single `#[test]` rather than eight independent ones — see the
comment at the top of the file for why (short version: the struct/
global tables are deliberate process-lifetime shared state, same as
in the original, and Rust's default parallel test execution would
race on it otherwise).

I can't claim exhaustive coverage of a ~2,660-line compiler from 8
programs, but the set was chosen to hit every branch I was least
confident about translating exactly (character-vs-byte indexing on
strings containing the UAGL Unicode symbol set, the immediate-encoding
bit arithmetic, the container's cryptographic validation), and all of
it matched.

## Behavioral notes: quirks preserved, not fixed

Converting to Rust surfaced a few things worth flagging. I kept the
original's exact behavior in every case rather than making a judgment
call to "fix" it — that seemed like the appropriate scope for a
language port — but you should know about them:

1. **Pointer-typed struct fields are sized as inline structs, not as
   8-byte pointers.** In `_type_size_align`, the leading `@` is
   stripped before the struct-name lookup, so a field declared like
   `next ─ @Node` gets sized as a full inline `Node`, not as a
   pointer. This is scoped to *struct fields* specifically —
   `type_size()` (used for local/parameter allocation) separately
   guards against it. Confirmed still present via `tests/fixtures/
   test5_edge.uagl` (a self-referential `Node.next: @Node` field),
   which both implementations compile identically (and which also
   exercises the layout cache's self-reference cycle-breaker, since a
   naive recursive implementation would infinite-loop on exactly this
   shape — see the doc comment on `compute_struct_layout` in
   `types.rs` for how that's preserved too).
2. **A very negative integer immediate (more negative than `-0x8000`)
   only gets its low 32 bits materialized.** In the large-immediate
   `MOVZ`/`MOVK` path, the guards on the `LSL #32`/`LSL #48` lines
   compare the *original signed value* against `0xFFFFFFFF` /
   `0xFFFFFFFFFFFF`, which is never true for a negative number — so
   those `MOVK` lines are skipped even when the two's-complement bit
   pattern needs them. Confirmed via `test5_edge.uagl`'s
   `-4294967297` literal.
3. **Two unreachable-but-present code paths**, kept for structural
   parity since removing them wasn't obviously in scope: `emit_load_value`'s
   "8. Function call result" branch (branch 6b already handles every
   input that would reach it), and `emit_scope`'s cache-flush branch
   (the SP-set branch just above it matches the identical `^\w+$`
   shape and always returns first).
4. **`emit_scope`'s local-variable offset checks use Python
   truthiness** (`if off:`) instead of `is not None`, so a destination
   local at stack offset **0** — the first local allocated in a
   function — is treated the same as "not a local." This is
   inconsistent with the rest of the file (which correctly uses
   `is not None` throughout) but is what the source does today.
5. **Two functions (`reg_for`, `emit_comparison`) and two more
   (`_fp_ldr_lines`/`_fp_str_lines`, `_struct_field_type`/
   `_array_field_stride` as generator methods) are defined but never
   called anywhere in the file.** All are ported and exposed on the
   Rust side for parity with the original's public surface, marked
   `#[allow(dead_code)]` where Rust would otherwise warn.

None of these are things I noticed and quietly patched — they're
called out here so the port is exactly as trustworthy (and exactly as
imperfect, in the same specific spots) as the file it's replacing.

## Design notes specific to the Rust translation

- **Global compiler state** (`STRUCT_TABLE`, `GLOBAL_TABLE`,
  `STRUCT_LAYOUTS`) is `Mutex`-guarded `static`s rather than function
  parameters, to preserve the documented "accumulates across every file
  compiled in one process" invariant — including under
  `assemble_batch`'s multi-threaded compilation, where Rust's `Mutex`
  takes over the job the GIL was implicitly doing.
- **`FuncState::label_counter` is a `Cell<i64>`**, not a plain field.
  Tracing every call site showed that the expression-codegen call graph
  (`emit_load_value`, `emit_call`, `_operand_aarch64_type`,
  `_emit_struct_base_addr`, `emit_comparison`, `emit_scope`) only ever
  *reads* `FuncState` — with the single exception of `emit_call`'s
  `__memcpy`/`__memset` builtins bumping the label counter. Modeling
  just that one field with interior mutability lets `Option<&FuncState>`
  (a `Copy` value) flow through that whole call graph unchanged, instead
  of threading `Option<&mut FuncState>` reborrows through dozens of
  recursive call sites for the sake of one counter.
- Several helpers do **character-position** scanning and slicing
  (`_strip_outer_parens`, `_split_binop`, `_split_call_args`,
  `_decode_uagl_string`, `_operand_aarch64_type`'s paren-stripping
  loop), because Python indexes strings by Unicode code point and UAGL
  source routinely mixes ASCII operators with multi-byte symbols
  (`─ △ ○ Φ ⁿ ═╗ ╚═══` etc.). These are ported using `Vec<char>`
  throughout rather than byte slicing, to keep the same index
  arithmetic meaning the same thing.
- One regex, `_INLINE_COMMENT_RE`, used a negative lookahead
  (`(?:\*(?!/)[^*]*)*`) that Rust's `regex` crate can't express (by
  design, to keep linear-time matching guarantees). It's replaced with
  a non-greedy `.*?`, which is a standard, exactly-equivalent rewrite
  for "shortest span up to the first closing delimiter" — verified via
  `test5_edge.uagl`'s block-comment cases. Every other regex in the
  file has no lookaround or backreferences and translates as-is.
- `decode_uaglbr_bytes` parses metadata as a generic `serde_json::Value`
  rather than a fixed struct, and re-derives Python's
  `json.dumps(ensure_ascii=True, sort_keys=True, separators=(",",":"))`
  formatting by hand (`canonical_json_value` in `uaglbr.rs`) rather than
  relying on a library default. This matters because the canonical-form
  byte comparison happens *before* the type-checking loop for several
  fields, so — matching the original — a malformed container needs to
  be representable (and correctly re-serialized) as whatever JSON shape
  it actually is, not assumed to already match the expected schema.
