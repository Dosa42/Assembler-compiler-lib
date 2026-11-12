# Assembler-compiler-lib

Build the imported GNU, LLVM/Clang and Zig source trees into executables and libraries on GitHub-hosted Ubuntu 24.04 ARM64 runners.

## Imported sources

- GNU: [GCC](sources/gcc), [Binutils/GDB](sources/binutils-gdb), [Newlib](sources/newlib-cygwin) and [Glibc](sources/glibc).
- [LLVM monorepo](sources/llvm-project): complete source tree, including Clang, LLD, LLDB, MLIR, Flang and the runtime libraries.
- [Zig](sources/zig): complete source tree imported from the project's repository on Codeberg.
- [Rust](sources/rust): compiler, standard library, Cargo, Clippy, rustfmt and the pinned upstream submodule sources.
- [rustup](sources/rustup): Rust toolchain installer and manager.
- WebAssembly: [wasm-tools](sources/wasm-tools), [WABT](sources/wabt), [Binaryen](sources/binaryen), [WASI SDK](sources/wasi-sdk), [Emscripten](sources/emscripten) and [emsdk](sources/emsdk).

Exact upstream URLs, branches, commits and source verification results are recorded in [sources/upstreams.json](sources/upstreams.json).

Rust and WebAssembly sources were imported on a GitHub-hosted `ubuntu-24.04-arm` runner, including their recursively pinned Git submodules. Each submodule is expanded into ordinary tracked source files at its original path. Every original file blob, executable mode and symlink is preserved. The manifest records both the upstream root tree and the expanded imported tree, with each submodule's URL, commit and verified tree. Rust's compiler-specific LLVM fork and WASI SDK's LLVM dependency retain their own required revisions.

The executable workflows currently cover GNU, LLVM and Zig. Rust, rustup and WebAssembly sources are ready for subsequent builds.

The [successful source-import run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43176726173) is retained as execution evidence. The [Rust/WebAssembly import workflow](.github/workflows/import-rust-wasm.yml) remains available through **Run workflow** as a reusable template. It skips already imported sources after checking their tree hashes. For WASI SDK's GNU config dependency, the pinned commit was retrieved from a GitHub mirror after Savannah returned HTTP 500; the manifest retains the upstream URL and records the fetch URL.

## Build and download

Open [Build GNU ARM64 executables](https://github.com/Dosa42/Assembler-compiler-lib/actions/workflows/build-gnu-arm64.yml), choose **Run workflow**, and select `all` or one component.

For LLVM and Zig, open [Build LLVM and Zig ARM64 executables](https://github.com/Dosa42/Assembler-compiler-lib/actions/workflows/build-llvm-zig-arm64.yml) and select `all`, `llvm` or `zig`.

The workflows also build changes to their source directories pushed to `main`. Both support `workflow_call`.

Each successful component uploads an artifact containing an installation archive and its SHA-256 checksum:

| Component | Artifact | Contents |
| --- | --- | --- |
| GCC | `gcc-ubuntu-24.04-arm64` | C/C++ compiler, compiler tools and runtime libraries |
| Binutils | `binutils-ubuntu-24.04-arm64` | ARM64 assembler, linker, readelf, objdump, objcopy, nm and strip |
| GDB | `gdb-ubuntu-24.04-arm64` | GDB and gdbserver |
| Newlib | `newlib-ubuntu-24.04-arm64` | Headers and static libraries for `aarch64-none-elf` |
| Glibc | `glibc-ubuntu-24.04-arm64` | ARM64 Linux C library, dynamic loader, headers and utilities |
| LLVM | `llvm-ubuntu-24.04-arm64` | LLVM tools, Clang, LLD, LLDB, clangd, clang-tidy, clang-format and development libraries; all default LLVM target backends |
| Zig | `zig-ubuntu-24.04-arm64` | Source-built stage3 Zig compiler, standard library and bundled LLVM 23 runtime dependencies |

Build logs are uploaded separately, including on failure. Installation artifacts are retained for 30 days and logs for 14 days.

## Use the executables

The executables run on ARM64 GNU/Linux. The build host is `ubuntu-24.04-arm`, and the Linux target is `aarch64-linux-gnu`. Ubuntu provides `gcc` and `aarch64-linux-gnu-gcc` to build the imported sources.

Download the artifact ZIPs from a completed run and unpack the ZIPs. Extract the GCC, Binutils and GDB installation archives into the same directory:

```sh
mkdir -p "$HOME/gnu-arm64"
tar -xJf gcc-ubuntu-24.04-arm64.tar.xz -C "$HOME/gnu-arm64"
tar -xJf binutils-ubuntu-24.04-arm64.tar.xz -C "$HOME/gnu-arm64"
tar -xJf gdb-ubuntu-24.04-arm64.tar.xz -C "$HOME/gnu-arm64"
export GNU_ARM64_ROOT="$HOME/gnu-arm64/opt/gnu"
export PATH="$GNU_ARM64_ROOT/gcc/bin:$GNU_ARM64_ROOT/binutils/bin:$GNU_ARM64_ROOT/gdb/bin:$PATH"
export LD_LIBRARY_PATH="$GNU_ARM64_ROOT/gcc/lib64:$GNU_ARM64_ROOT/gcc/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
gcc --version
g++ --version
aarch64-linux-gnu-as --version
gdb --version
```

Extract LLVM and Zig into the same installation root:

```sh
tar -xJf llvm-ubuntu-24.04-arm64.tar.xz -C "$HOME/gnu-arm64"
tar -xJf zig-ubuntu-24.04-arm64.tar.xz -C "$HOME/gnu-arm64"
export PATH="$HOME/gnu-arm64/opt/llvm/bin:$HOME/gnu-arm64/opt/zig/bin:$PATH"
clang --version
lldb --version
zig version
```

These are dynamic executables and use the corresponding Ubuntu runtime libraries. Glibc's archive contains a separate sysroot. Newlib's archive is for bare-metal AArch64 development.

The LLVM build enables `clang;clang-tools-extra;lld;lldb` and every default LLVM target backend. The imported LLVM source is version 24. Zig's imported source requires LLVM 23 development libraries, which the GitHub build obtains from the official apt.llvm.org repository. Zig itself is bootstrapped and compiled from the imported source. Its runtime dependency libraries are bundled with relative runtime paths.

## Verification and provenance

Every successful artifact has passed a component-specific check:

- GCC compiles and executes C and C++ programs.
- Binutils assembles and links an executable, then executes it.
- GDB starts a debuggee, hits a breakpoint and evaluates an expression.
- Newlib links a bare-metal AArch64 ELF against its newly built libc.
- Glibc runs an executable with its newly built dynamic loader and libc.
- LLVM/Clang compile and execute C and C++; LLVM IR passes through the assembler, optimizer, code generator and archiver into an executed program. LLDB debugs a program and executes Python.
- Zig compiles and executes Zig, C and C++, runs Zig tests with its LLVM backend, and cross-compiles an x86_64 ELF object.

The archives include a build report, upstream commit manifest, source location and license files inside the component's installation prefix: `share/gnu-build` for GNU, and `share/build-info` for LLVM and Zig.

The source revisions are pinned in [sources/upstreams.json](sources/upstreams.json). Binutils and GDB share the imported `sources/binutils-gdb` tree.

## Reuse the build workflows

```yaml
name: Build GCC
on: workflow_dispatch
permissions:
  contents: read
jobs:
  gcc:
    uses: ./.github/workflows/build-gnu-arm64.yml
    with:
      component: gcc
```

GCC is built with C and C++, without multilib or a three-stage bootstrap. The workflow runs the checks listed above; complete upstream regression suites are separate.

For LLVM or Zig, call `./.github/workflows/build-llvm-zig-arm64.yml` with `component: llvm`, `component: zig` or `component: all`.
