# wasm-bindgen for ubuntu-24.04-arm64

The installation archive contains this component's source-built executables, libraries and build provenance.

[Original build run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43265042785)

Source repository commit: 8a7bd94613ed0b14754da1b18ee155a82eaded29.

## Files

- [wasm-bindgen-ubuntu-24.04-arm64.tar.xz](wasm-bindgen-ubuntu-24.04-arm64.tar.xz) (5650356 bytes)

Download the files above, SHA256SUMS and wasm-bindgen-ubuntu-24.04-arm64.tar.xz.sha256 into one directory.

```sh
sha256sum -c SHA256SUMS
sha256sum -c wasm-bindgen-ubuntu-24.04-arm64.tar.xz.sha256
mkdir -p ./toolchains
tar -xJf wasm-bindgen-ubuntu-24.04-arm64.tar.xz -C ./toolchains
```

These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.
