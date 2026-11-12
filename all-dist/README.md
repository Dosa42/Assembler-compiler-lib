# Built distributions

Source-built installation archives committed to this repository. Each archive contains the executables, libraries and build information produced by its verified GitHub build.

| Component | Platform | Build run | Files |
| --- | --- | --- | --- |
| [binutils](ubuntu-24.04-arm64/binutils/README.md) | ubuntu-24.04-arm64 | [43163341859](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43163341859) | [binutils-ubuntu-24.04-arm64.tar.xz](ubuntu-24.04-arm64/binutils/binutils-ubuntu-24.04-arm64.tar.xz) |
| [gcc](ubuntu-24.04-arm64/gcc/README.md) | ubuntu-24.04-arm64 | [43163341859](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43163341859) | [gcc-ubuntu-24.04-arm64.tar.xz](ubuntu-24.04-arm64/gcc/gcc-ubuntu-24.04-arm64.tar.xz) |
| [gdb](ubuntu-24.04-arm64/gdb/README.md) | ubuntu-24.04-arm64 | [43163341859](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43163341859) | [gdb-ubuntu-24.04-arm64.tar.xz](ubuntu-24.04-arm64/gdb/gdb-ubuntu-24.04-arm64.tar.xz) |
| [glibc](ubuntu-24.04-arm64/glibc/README.md) | ubuntu-24.04-arm64 | [43163744598](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43163744598) | [glibc-ubuntu-24.04-arm64.tar.xz](ubuntu-24.04-arm64/glibc/glibc-ubuntu-24.04-arm64.tar.xz) |

## Download and restore

Run the helper with Python 3. It downloads a component, verifies each SHA-256 checksum and restores its original installation archive. Add --extract to unpack the installation tree.

```sh
curl -fL https://raw.githubusercontent.com/Dosa42/Assembler-compiler-lib/main/all-dist/restore.py -o restore.py
python3 restore.py gcc --output ./downloads --extract ./toolchains
python3 restore.py zig --output ./downloads --extract ./toolchains
```

You can also download the files directly using the links above. Archives larger than 95 MiB are stored in numbered parts. Each component directory contains its SHA-256 checksums, provenance and reconstruction instructions.
