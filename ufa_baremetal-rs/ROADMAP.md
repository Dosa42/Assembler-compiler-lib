# Roadmap — ufa_baremetal-rs

Ranked by impact first, not by how fun or easy each one is. "Impact" here
means: how much sooner does a real K-violation get caught, for how much
engineering cost and risk. Two items (SARIF, editor integration) both
build on the `--json` schema that already exists, so neither requires
touching the checking core.

| # | Item | Impact | Effort | Risk |
|---|---|---|---|---|
| 1 | Parallelize directory checks | Speed, at scale | Low | None |
| 2 | SARIF output | Bugs flagged on the PR diff | Medium | Low |
| 3 | `--watch` mode | Tighter local dev loop | Low–Medium | Low |
| 4 | Editor integration (LSP) | Bugs flagged as you type | High | Medium |
| 5 | New K-checks (ongoing) | The tool's actual bug-catching power | Varies | Low, if scoped like K01–K06 |
| 6 | WASM / browser playground | Onboarding & demos | Medium | Low |
| 7 | `--fix` mode | Removes manual fix-up work | Medium–High | **High** |

---

## 1. Parallelize directory checks — ✅ done

**What:** `audit_directory` currently checks files sequentially. Each
file's checks are already pure and fully independent — no shared state,
no ordering requirement — so this is a textbook `std::thread` (or a
thread pool) fan-out, no new dependency needed.

**Why first:** It's the only item on this list with real impact and
*zero* downside. It doesn't touch check logic at all, so nothing that's
already been verified against the Python original is at risk. On a
large `.s` tree in CI, this is a direct wall-clock win with an
afternoon of work.

**Depends on:** nothing.

**Status:** Implemented as a work-stealing pool of `std::thread` workers
(shared `AtomicUsize` cursor over the sorted file list, no new
dependency), bounded by `std::thread::available_parallelism()`. Output
order and content are identical to the old sequential version regardless
of thread scheduling — results are tagged with their original sorted
index and re-sorted before being folded into the returned `BTreeMap`.
Covered by two new tests: one cross-checking the parallel path against
direct per-file `audit_file` calls, one asserting repeated runs on the
same directory produce byte-identical output.

---

## 2. SARIF output

**What:** A `--sarif` flag (or a second branch of the existing
`_emit_json`-equivalent path) that emits [SARIF 2.1.0](https://sarifweb.azurewebsites.net/)
instead of the current ad-hoc JSON schema. GitHub code scanning (and
most other static-analysis dashboards) understand SARIF natively —
FAILs would show up as inline annotations directly on the PR diff,
where the change was introduced.

**Why this high:** Best leverage-per-hour on the list. It reuses
everything `Verdict` already carries (status, file, line, diagnostic,
suggested fix) — this is a new serializer, not new analysis. And
"annotation on the PR" is most of the value of a full editor extension
(#4) for a fraction of the engineering and *zero* new runtime
architecture (no long-running process, no protocol).

**Depends on:** the existing `Verdict` → structured-output path (already
built for `--json`).

---

## 3. `--watch` mode

**What:** Re-run checks automatically when a watched file or directory
changes, printing a fresh report each time — the classic tight local
loop (`cargo watch`-style) for iterating on the compiler backend without
manually re-invoking the CLI.

**Why here:** Cheap relative to its value, and it's genuinely useful
standalone even before editor integration exists. Two implementation
options: pull in a filesystem-notify crate (small, well-established,
but breaks the core+CLI's zero-dependency property), or keep it
dependency-free with a simple poll loop (`mtime` diffing every N ms) —
worth deciding deliberately rather than defaulting to the crate.

**Depends on:** nothing structurally; ordered after SARIF because SARIF
has a better impact/effort ratio, not because of a technical dependency.

---

## 4. Editor integration (as an LSP, not a one-off extension)

**What:** A small Language Server wrapping the existing check pipeline,
so K-violations show up as native diagnostics (squiggles, hover text,
quick-fixes from `suggested_fix`) *as you type* — before save, before
commit, before CI. Building it as an LSP rather than a VS Code-specific
extension means one implementation works in VS Code, Neovim, Helix, or
anything else with an LSP client, instead of re-implementing per editor.

**Why not #1:** It's genuinely the highest per-catch impact — the
earliest possible point to catch a bug is while writing the line that
introduces it — but it's the most architecturally different piece of
work here (a long-running process watching files and speaking a
protocol, not a batch CLI invocation), so it's ranked below the cheaper
wins that should land first.

**Depends on:** can reuse `--watch`'s file-change-detection logic;
otherwise independent of SARIF.

---

## 5. New K-checks (ongoing, not a single milestone)

**What:** K07 and beyond, following the existing discipline — one
function in `k_invariants.rs`, one `K_REGISTRY` entry, no subclass
hierarchy, no plugin system.

**Why it's here and not #1:** This is arguably the *most* valuable work
on the list — it's literally the tool's bug-catching power — but it
isn't a schedulable engineering phase the way the others are. New checks
get added in response to real bugs found in the compiler, the same way
K01–K06 trace back to specific numbered bugs in the regression table.
Treat this as a standing practice that runs in parallel with everything
else here, not a phase to get to eventually.

**Depends on:** nothing; can start immediately, any time a new bug shape
is found.

---

## 6. WASM / browser playground — ✅ done

**What:** Compile `ufa_baremetal_core` to `wasm32-unknown-unknown` (it
already has zero external dependencies, so this should be close to a
clean cross-compile) and wrap it in a minimal page: paste `.s`, see
verdicts, no server, no install.

**Why mid-table:** Genuinely useful for onboarding new compiler-team
engineers and for demos, but it doesn't make anyone's day-to-day loop
faster the way #1–#4 do. Good "spare cycle" project.

**Depends on:** the core staying dependency-free (already true — this is
exactly the kind of payoff that discipline was for).

**Status:** The cross-compile was indeed clean — `ufa_baremetal_core`
as-is (including `codegen_audit.rs`'s `std::thread`/`std::fs` usage from
item #1's work) compiles for `wasm32-unknown-unknown` with no changes.
What the core didn't have was a filesystem-free entry point: `audit_file`
takes a `Path` and calls `fs::read_to_string`, neither of which makes
sense for pasted text with no disk file behind it. Fixed by factoring
`audit_file`'s body into two new pure functions,
`audit_text`/`audit_text_named` — `audit_file` now just reads the file
and delegates. Same behavior, confirmed byte-identical by the existing
golden tests (unchanged, still passing) plus three new unit tests in
`codegen_audit.rs`, one of which directly asserts `audit_file` and
`audit_text_named` produce identical `Vec<Verdict>` output for the same
bytes. This also sets up item #4 (LSP): an editor's unsaved buffer is
exactly the same "text with no real path" shape.

New crate: `bin/ufa-baremetal-wasm`, a `cdylib` exporting three
hand-rolled `extern "C" fn`s (`alloc`/`dealloc`/`check` — pointers and
lengths, no `wasm-bindgen`) over `audit_text`, plus `www/`, a
self-contained playground page (`build.sh` embeds the compiled `.wasm`
into `index.html` as base64, so the page needs no `fetch()` and works
opened straight from disk). It's its own standalone Cargo workspace, not
a member of the root one — see the README's WASM section for why
(`panic = "abort"` in its release profile, which Cargo won't honor from
a non-root member, and which shouldn't be forced onto the CLI/GUI's
build by putting it in the shared root profile instead).

One real bug caught by actually building and running this rather than
just getting it to compile: K05 only fires for files named `*_boot.s` /
`*_init.s`. An early version of `check()` hardcoded one filename
constant for every call, which meant K05 could never fire from the
browser no matter what was pasted — a silently dead sixth of the tool's
checks. Fixed by making the filename a real parameter, JS-side field
included, with a visible hint about the naming rule right next to it.

Verified end-to-end, not just "it compiles": the actual compiled
`.wasm` was loaded in a real WASM runtime (Node's, same V8 engine
Chrome uses) and driven through the exact `alloc`/`check`/`dealloc`
sequence the page's JS uses, confirming output byte-for-byte equivalent
to the native CLI's `--json` output for the same input (modulo the
`file` key, omitted on the WASM path since the caller already supplies
that name). The actual generated `www/index.html` — real DOM, real
inline `<script>`, the real embedded module — was then driven through a
full user flow (load example → Check → click a diagnostic's line number
→ jump-and-highlight; the parse-completeness error path; the K05
filename-gating fix) via jsdom, catching the filename-gating bug above
and a `bash`/Linux argv-length limit in an earlier draft of `build.sh`
(a single shell argument over ~128KB fails with a bare "Argument list too
long" on Linux — the base64 payload is embedded by having Python read
and encode the `.wasm` file directly instead of round-tripping the
encoded text through a shell variable and command-line argument).

---

## 7. `--fix` mode

**What:** For the subset of `suggested_fix` text that's fully mechanical
(e.g. K05's exact four-instruction barrier sequence has one correct
form), offer to apply it directly to the `.s` file — `--dry-run` by
default, real diff preview before anything is written, opt-in
`--write`.

**Why last:** Not because it's low-value — because it's the only item
here with real *downside*. Every other item can only fail to help; this
one can actively corrupt someone's working tree if a check (or the fix
logic itself) has a bug. It deserves a track record of the checker being
trusted in production first, plus its own design pass on safety:
idempotency (running it twice shouldn't double-insert anything),
interaction with uncommitted changes, and which of the six checks even
have a fix mechanical enough to automate (K05's is; K03's SMULL/UMULL
rewrite arguably is too; K01's frame-size math is riskier to auto-edit).

**Depends on:** maturity/trust in the underlying checks — don't
auto-apply a fix generated by a check that might itself be wrong.
