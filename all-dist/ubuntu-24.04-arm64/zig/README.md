# zig for ubuntu-24.04-arm64

The installation archive contains this component's source-built executables, libraries and build provenance.

[Original build run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43168282553)

Source repository commit: f8baaa2a81a99f8ad72cd4a62c49f3d2a7839105.

## Files

- [zig-ubuntu-24.04-arm64.tar.xz](zig-ubuntu-24.04-arm64.tar.xz) (95713496 bytes)

Download the files above, SHA256SUMS and zig-ubuntu-24.04-arm64.tar.xz.sha256 into one directory.

```sh
sha256sum -c SHA256SUMS
sha256sum -c zig-ubuntu-24.04-arm64.tar.xz.sha256
mkdir -p ./toolchains
tar -xJf zig-ubuntu-24.04-arm64.tar.xz -C ./toolchains
```

These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.
