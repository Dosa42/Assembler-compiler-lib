# ufa_baremetal (Rust port)

A Rust port of the `ufa_baremetal` Python package: the K01–K06 invariant
checks that verify emitted AArch64 assembly against the UFA Bare-Metal
Canon brief. Ported module-for-module from the Python source, plus a
native GUI wrapped around the same checking core.

## Offline playground

Open `ufa-baremetal-playground.html` from disk. Load C, UAGL, UAGLBR or
AArch64 assembly, choose a preset or configure steps in **Custom**, then
select **Run pipeline**. Each step can use a loaded file or compatible
output from an earlier step. Presets cover C and UAGL conversion,
K-checks on supplied assembly first, and fidelity and reverse conversion.
Pipeline JSON stores the configuration. Input files are supplied separately.

By default a run continues after diagnostics or a failed step if the
next step still has its inputs. Choose **Stop bij fouten** to stop at the
first failed step. Step results, source files and generated files remain
available. **Verify** compares assembly with a reference `.s` file and
reports line differences. K05 uses the assembly filename.

The converter's `.gaps.json` remains available alongside a combined
diagnostics export. The combined export identifies each tool and keeps
source locations when the tool supplies them.

### Audit cases

| Input or operation | Observed behavior | Where it is visible |
|---|---|---|
| Prefix cast such as `(INT64)x` | The backend emits an unresolved-expression comment. | Assembler source check and the combined diagnostics. |
| `INT128` locals | The backend allocates eight-byte slots. | Inspection of generated assembly. The current source check can return no findings. |
| Flat and nested initializers | A flat multiline numeric array works. A nested aggregate can emit zero values. | Compare emitted data with the reference. The current source check can return no findings. |
| `SIZEOF ○ UINT8` | The backend emits a call to `SIZEOF`. | Assembly inspection and symbol resolution during linking. |
| `UINT64 × UINT64` with a `UINT64` result | A valid `MUL X` can be flagged as requiring `UMULL`. | Compare actual operand widths and execution before changing the instruction. |
| `UINT32 × UINT32` assigned to `UINT64` | An emitted `UMULL` can keep bits that the C multiplication discards before assignment. | Compare execution against C. K03 can pass. |

Verify checks assembly text against a supplied reference. Behavioral
comparison is a separate workflow that assembles and links the `.s` for
AArch64, runs it, and compares its results with the original C.

In a separate check with GNU AArch64 binutils 2.44 and QEMU 10.0.13,
small 64-bit addition and multiplication cases matched native C. For
`UINT32_MAX × 2` assigned to `UINT64`, native C produced `4294967294`
and the emitted AArch64 produced `8589934590`, despite a K03 pass.
The full Ed25519 assembly produced an object file. Linking stopped on
unresolved symbols including `SIZEOF`, `_ge_B`, and SHA-512 functions.
The supplied C source also needs `ed25519_full.h` and `crypto_d2.h`
to build a native reference executable.

## Status

| Piece | Status |
|---|---|
| `lib/ufa_baremetal_core` — K01–K06 checks, parser, verdicts, regression map | **Done.** Zero external dependencies. |
| `bin/ufa-baremetal` — CLI (`--json`, `--summary`, `--k`, `bug-coverage`) | **Done.** Zero external dependencies. |
| `bin/ufa-baremetal-gui` — native GUI (egui/eframe) | **Done.** One external dependency (egui/eframe — see below). |
| `bin/ufa-baremetal-wasm` — browser checker (roadmap #6) | Uses `ufa_baremetal_core` to check assembly in the playground. |
| `bin/uagl-assembler-wasm` — browser assembler | Uses the `uagl_aarch64_assembler` library. |
| `c-uagl-rust` — C converter | Supplies C → UAGL, reverse conversion and fidelity to the same offline page. |

## Why this exists / how it was built

This is a line-for-line port, not a rewrite: every K-check, every regex
(hand-translated into small parser functions — see below), and the full
65-entry bug-regression table were checked against the real Python
implementation before being committed here, because this sandbox has no
Rust compiler available to catch mistakes by compiling. Concretely, that
meant:

- Every regex in `parse_s.py`/`k_invariants.py` was run against
  representative input in Python first to pin down its exact match
  behavior, then hand-translated into a small dedicated matcher function
  in `pattern.rs`/`parse_s.rs` (see the module docs there for why this
  crate doesn't depend on the `regex` crate).
- The full K01–K06 pipeline was run against 13 hand-built `.s` fixtures
  through the actual Python package, and the exact verdict text
  (including line numbers) was captured and turned into the integration
  tests in `lib/ufa_baremetal_core/tests/golden.rs` — generated
  programmatically from the Python output, not transcribed by hand.
- SHA-256, the JSON writer, and the UTC-timestamp/calendar math (all
  hand-rolled to keep the core dependency-free) were each checked against
  Python's `hashlib`/`json`/`datetime` output, including padding-boundary
  and leap-year edge cases.
- The `BUG_TO_K` regression table (65 entries) was diffed field-by-field
  against the Python dict programmatically; this caught a real
  transcription mistake (a wrong `detection` field on ~36 entries) before
  it shipped.

None of this required a Rust compiler — it's all Python-side
cross-checking of the *logic* before transcribing it. It does not
replace actually running `cargo build && cargo test` yourself, which I
could not do here (see below).

## Building

```sh
cd ufa_baremetal-rs

# Core + CLI (no external dependencies, offline-buildable):
cargo build --release --workspace --exclude ufa-baremetal-gui
cargo test --workspace --exclude ufa-baremetal-gui   # golden-output test suite
cargo run --bin ufa-baremetal -- path/to/file.s

# Everything, including the GUI (needs crates.io reachability the first
# time, to fetch egui/eframe):
cargo build --release --workspace
cargo run --bin ufa-baremetal-gui
```

**I could not run any of this myself.** This sandbox has no Rust
toolchain installed and no network access to crates.io, so I was unable
to `cargo build`, `cargo test`, or `cargo check` anything in this
project — not even the dependency-free core. Everything above was
validated by hand-tracing the translated logic against real Python
output (see previous section), which is a real but different kind of
check than actually compiling it. If `cargo build` turns up a syntax or
type error, it's very possible — please paste me the compiler output and
I'll fix it immediately; that class of error is exactly what a compiler
catches and cross-checking logic against Python can't. The GUI crate is
the highest-risk piece in that respect, since egui/eframe's API moves
between minor versions and I pinned `0.35` (current as of when this was
written, confirmed via web search — see `bin/ufa-baremetal-gui/Cargo.toml`)
without being able to compile against it even once.

### CLI usage

```sh
ufa-baremetal path/to/file.s                 # print FAILs
ufa-baremetal path/to/dir --summary          # + per-file PASS/FAIL/N-A counts
ufa-baremetal path/to/file.s --show-pass     # also print PASS/N-A
ufa-baremetal path/to/file.s --k K01,K05     # restrict to a subset
ufa-baremetal path/to/file.s --json          # machine-readable output
ufa-baremetal bug-coverage                   # print the regression-table coverage report
```

Exit codes: `0` clean, `1` at least one FAIL, `2` usage/IO/parse error.

### GUI

A workbench, not just a results viewer:

- Type a path (file or directory) and click **Run**, or just **drag a
  `.s` file/folder onto the window**.
- Per-K-class checkboxes (K01–K06) filter what's displayed — the checker
  still runs all six; this only controls the view.
- Toggle PASS / NOT_APPLICABLE visibility independently; FAILs always show.
- Results are grouped per file in collapsible sections (auto-expanded
  when a file has FAILs), each verdict showing its K-class, status,
  file:line, function, full diagnostic, and (for FAILs) the suggested fix
  — all with the same text your terminal would show, but with color and
  structure.
- Live PASS/FAIL/N-A summary and a **Copy results** button (respects the
  current filters).
- A second **Bug Coverage** tab renders the same regression-ledger report
  as `ufa-baremetal bug-coverage`, so the K-check results and the
  bug-to-K-class ledger sit side by side in one tool.

### WASM playground (roadmap #6)

```sh
rustup target add wasm32-unknown-unknown   # one-time
./bin/ufa-baremetal-wasm/build.sh
```

`build.sh` uses Cargo and Python 3 to embed the converter, assembler and
checker Wasm modules and the playground scripts in the root
`ufa-baremetal-playground.html`. The built page runs offline in the
browser. A rebuild needs the locked Cargo dependencies in the local
cache when run offline. The build also updates the standalone converter
and assembler modules in their respective `dist/wasm/` directories.

`bin/ufa-baremetal-wasm` is its own standalone Cargo workspace (an empty
`[workspace]` table in its `Cargo.toml`, not a member of the root
workspace) for one concrete reason: it needs `panic = "abort"` in its
release profile (smaller output, and sidesteps needing WASM's
exception-handling proposal just to link), and Cargo only reads
`[profile.*]` from a workspace *root*. Setting that at the root workspace
level instead would flip `panic = "abort"` onto the CLI and GUI too,
which would break `cargo test`'s ability to catch one test's panic and
keep running the rest. Being its own workspace also means touching it
can't affect `cargo build --workspace` at the repo root for anyone not
working on the playground.

The modules expose `alloc` and `dealloc` for the page's JavaScript. The
checker exports `check` and `check_selected`. The assembler exports
`assemble` and `validate` for UAGL text or raw `.uaglbr` bytes. The
converter exports `convert_c`, `uagl_to_c` and `fidelity_report`. Each
operation returns UTF-8 JSON through a pointer and length packed into a
single `u64`.

Two things worth knowing if you're extending this:

- **The filename you type in the playground isn't cosmetic.** K05 only
  runs for names ending in `_boot.s` or `_init.s` — same rule as the CLI,
  checked against this same string. An earlier draft of this hardcoded
  one constant name for every check, which made K05 permanently
  unreachable from the browser; the field exists because that's the kind
  of bug that's easy to ship silently in a playground nobody's writing a
  golden test against.
- **`audit_text`/`audit_text_named`** (new, small additions to
  `codegen_audit.rs`) are what make this possible at all: a filesystem-free
  equivalent of `audit_file` that takes text already in memory instead of
  a `Path`. `audit_file` itself now just reads the file and delegates to
  it — same behavior, verified byte-identical against the old
  implementation by the golden tests plus three new unit tests. This
  matters beyond the WASM path, too: item #4 on the roadmap (an LSP) will
  want the same thing for unsaved editor buffers.

## Layout

Follows the `lib/`+`bin/` convention already established elsewhere in
this project's Rust efforts (see brief §8.6):

```
ufa_baremetal-rs/
├── lib/ufa_baremetal_core/     # the checking engine (no external deps)
│   ├── src/
│   │   ├── verdict.rs          # Status/Verdict — port of verdict.py
│   │   ├── pattern.rs          # hand-rolled regex-replacement primitives
│   │   ├── parse_s.rs          # port of parse_s.py
│   │   ├── k_invariants.rs     # port of k_invariants.py — K01-K06
│   │   ├── codegen_audit.rs    # port of codegen_audit.py
│   │   ├── regression_map.rs   # port of regression_map.py — BUG_TO_K table
│   │   ├── json.rs             # hand-rolled JSON writer (for --json)
│   │   ├── sha256.rs           # hand-rolled SHA-256 (for --json)
│   │   └── datetime.rs         # hand-rolled UTC ISO-8601 (for --json)
│   └── tests/golden.rs         # integration tests vs. real Python output
├── bin/ufa-baremetal/          # CLI — port of cli.py + __main__.py
├── bin/ufa-baremetal-gui/      # native GUI (egui/eframe)
│   └── src/
│       ├── main.rs              # NativeOptions + run_native
│       └── app.rs               # UfaApp: state + UI (the "workbench")
└── bin/ufa-baremetal-wasm/     # browser playground (roadmap #6) — its own
    │                           # standalone Cargo workspace, see README above
    ├── src/lib.rs               # alloc/dealloc/check — the WASM ABI
    ├── build.sh                 # rebuilds the root ufa-baremetal-playground.html
    └── www/
        ├── template.html.in      # editable page template
        ├── pipeline-core.js      # step definitions and execution
        ├── wasm-tools.js         # bindings for the three Wasm modules
        └── unified-ui.js         # editor and pipeline controls
```

## Design choices worth knowing about

- **Zero dependencies in `lib/` and `bin/ufa-baremetal`.** The Python
  brief's own non-negotiable (§5.1) is "stdlib only, no third-party deps
  beyond pytest"; this port keeps that discipline for the core and CLI,
  which also means a K-check pipeline can run in a locked-down/air-gapped
  embedded CI runner with only `cargo` and no crates.io reachability. The
  GUI is the one place an external dependency (`egui`/`eframe`) is
  unavoidable, since there's no GUI framework in `std`.
- **Directory checks run in parallel.** `audit_directory` fans work out
  across a small pool of `std::thread` workers (bounded by
  `std::thread::available_parallelism()`, no new dependency) since every
  file's checks are already pure and independent. Output is identical to
  a sequential run regardless of scheduling — see `ROADMAP.md` #1 for
  the design notes.
- **Faithfully preserved quirks, not "fixed."** A few places in the
  Python source have real, checked-in behavior that looks like it might
  be a bug (e.g., a function whose body silently excludes its last line
  when there's no `.size` marker — see the doc comment on
  `parse_functions` in `parse_s.rs`). This port preserves that behavior
  exactly rather than silently "improving" it, the same way
  `regression_map.py`'s own docstring says not to silently reclassify its
  bug table.
- **Vestigial regex alternatives, simplified with a note.** A handful of
  the Python regexes contain alternatives that can never actually fire
  (e.g. `(W\w+|WZR)` — `WZR` already matches `W\w+`). Where I found one of
  these, I simplified the matcher and left a comment explaining why the
  simplification is behavior-preserving, rather than carrying the dead
  branch forward.
