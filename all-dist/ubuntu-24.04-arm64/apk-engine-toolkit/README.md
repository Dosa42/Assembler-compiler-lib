# apk-engine-toolkit for ubuntu-24.04-arm64

The installation archive contains this component's source-built executables, libraries and build provenance.

[Original build run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/44119139955)

Source repository commit: 244623d2f4d26526213e046da8cc0caddf0fcfac.

## Files

- [apk-engine-toolkit-ubuntu-24.04-arm64.tar.xz](apk-engine-toolkit-ubuntu-24.04-arm64.tar.xz) (7950072 bytes)

Download the files above, SHA256SUMS and apk-engine-toolkit-ubuntu-24.04-arm64.tar.xz.sha256 into one directory.

```sh
sha256sum -c SHA256SUMS
sha256sum -c apk-engine-toolkit-ubuntu-24.04-arm64.tar.xz.sha256
mkdir -p ./toolchains
tar -xJf apk-engine-toolkit-ubuntu-24.04-arm64.tar.xz -C ./toolchains
```

These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.
