# wabt for ubuntu-24.04-arm64

The installation archive contains this component's source-built executables, libraries and build provenance.

[Original build run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43179370823)

Source repository commit: 24ea2a565e23022343bb4ed7de2a75d4341064b6.

## Files

- [wabt-ubuntu-24.04-arm64.tar.xz](wabt-ubuntu-24.04-arm64.tar.xz) (2367220 bytes)

Download the files above, SHA256SUMS and wabt-ubuntu-24.04-arm64.tar.xz.sha256 into one directory.

```sh
sha256sum -c SHA256SUMS
sha256sum -c wabt-ubuntu-24.04-arm64.tar.xz.sha256
mkdir -p ./toolchains
tar -xJf wabt-ubuntu-24.04-arm64.tar.xz -C ./toolchains
```

These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.
