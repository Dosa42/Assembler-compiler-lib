# glibc for ubuntu-24.04-arm64

The installation archive contains this component's source-built executables, libraries and build provenance.

[Original build run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43163744598)

Source repository commit: c960ee3bfe305dfebd2c3f284ddd55c5e1ae5bc1.

## Files

- [glibc-ubuntu-24.04-arm64.tar.xz](glibc-ubuntu-24.04-arm64.tar.xz) (11082312 bytes)

Download the files above, SHA256SUMS and glibc-ubuntu-24.04-arm64.tar.xz.sha256 into one directory.

```sh
sha256sum -c SHA256SUMS
sha256sum -c glibc-ubuntu-24.04-arm64.tar.xz.sha256
mkdir -p ./toolchains
tar -xJf glibc-ubuntu-24.04-arm64.tar.xz -C ./toolchains
```

These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.
