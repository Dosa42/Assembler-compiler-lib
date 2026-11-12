# Assembler-compiler-lib

Build the imported GNU source trees into executables and libraries on GitHub-hosted Ubuntu 24.04 ARM64 runners.

## Build and download

Open [Build GNU ARM64 executables](https://github.com/Dosa42/Assembler-compiler-lib/actions/workflows/build-gnu-arm64.yml), choose **Run workflow**, and select `all` or one component.

The workflow also builds imported source changes pushed to `main`. It can be called from another workflow with `workflow_call`.

Each successful component uploads an artifact containing an installation archive and its SHA-256 checksum:

| Component | Artifact | Contents |
| --- | --- | --- |
| GCC | `gcc-ubuntu-24.04-arm64` | C/C++ compiler, compiler tools and runtime libraries |
| Binutils | `binutils-ubuntu-24.04-arm64` | ARM64 assembler, linker, readelf, objdump, objcopy, nm and strip |
| GDB | `gdb-ubuntu-24.04-arm64` | GDB and gdbserver |
| Newlib | `newlib-ubuntu-24.04-arm64` | Headers and static libraries for `aarch64-none-elf` |
| Glibc | `glibc-ubuntu-24.04-arm64` | ARM64 Linux C library, dynamic loader, headers and utilities |

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

These are dynamic executables and use the corresponding Ubuntu runtime libraries. Glibc's archive contains a separate sysroot. Newlib's archive is for bare-metal AArch64 development.

## Verification and provenance

Every successful artifact has passed a component-specific check:

- GCC compiles and executes C and C++ programs.
- Binutils assembles and links an executable, then executes it.
- GDB starts a debuggee, hits a breakpoint and evaluates an expression.
- Newlib links a bare-metal AArch64 ELF against its newly built libc.
- Glibc runs an executable with its newly built dynamic loader and libc.

The archives include a build report, upstream commit manifest, source location and license files under `share/gnu-build` inside the component's installation prefix.

The source revisions are pinned in [sources/upstreams.json](sources/upstreams.json). Binutils and GDB share the imported `sources/binutils-gdb` tree.

## Reuse the build workflow

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
