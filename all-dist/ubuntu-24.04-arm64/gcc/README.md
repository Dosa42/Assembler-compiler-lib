# gcc for ubuntu-24.04-arm64

The installation archive contains this component's source-built executables, libraries and build provenance.

[Original build run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43163341859)

Source repository commit: 6765e7b071215c4bd3577ad41fb7b008cc5a6be4.

## Files

- [gcc-ubuntu-24.04-arm64.tar.xz](gcc-ubuntu-24.04-arm64.tar.xz) (79029668 bytes)

Download the files above, SHA256SUMS and gcc-ubuntu-24.04-arm64.tar.xz.sha256 into one directory.

```sh
sha256sum -c SHA256SUMS
sha256sum -c gcc-ubuntu-24.04-arm64.tar.xz.sha256
mkdir -p ./toolchains
tar -xJf gcc-ubuntu-24.04-arm64.tar.xz -C ./toolchains
```

These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.
