# llvm for ubuntu-24.04-arm64

The installation archive contains this component's source-built executables, libraries and build provenance.

[Original build run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43168282553)

Source repository commit: f8baaa2a81a99f8ad72cd4a62c49f3d2a7839105.

## Files

- [llvm-ubuntu-24.04-arm64.tar.xz.part0001](llvm-ubuntu-24.04-arm64.tar.xz.part0001) (99614720 bytes)
- [llvm-ubuntu-24.04-arm64.tar.xz.part0002](llvm-ubuntu-24.04-arm64.tar.xz.part0002) (99614720 bytes)
- [llvm-ubuntu-24.04-arm64.tar.xz.part0003](llvm-ubuntu-24.04-arm64.tar.xz.part0003) (28797800 bytes)

Download the files above, SHA256SUMS and llvm-ubuntu-24.04-arm64.tar.xz.sha256 into one directory.

```sh
sha256sum -c SHA256SUMS
cat llvm-ubuntu-24.04-arm64.tar.xz.part* > llvm-ubuntu-24.04-arm64.tar.xz
sha256sum -c llvm-ubuntu-24.04-arm64.tar.xz.sha256
mkdir -p ./toolchains
tar -xJf llvm-ubuntu-24.04-arm64.tar.xz -C ./toolchains
```

These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.
