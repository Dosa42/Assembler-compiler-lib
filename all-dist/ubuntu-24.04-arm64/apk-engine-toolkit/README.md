# apk-engine-toolkit for ubuntu-24.04-arm64

The installation archive contains this component's source-built executables, libraries and build provenance.

[Original build run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/44457163910)

Source repository commit: 5d3da36037adcda6cfe2caa9450f1d28cc6a2f55.

## Files

- [apk-engine-toolkit-ubuntu-24.04-arm64.tar.xz](apk-engine-toolkit-ubuntu-24.04-arm64.tar.xz) (9993692 bytes)

Download the files above, SHA256SUMS and apk-engine-toolkit-ubuntu-24.04-arm64.tar.xz.sha256 into one directory.

```sh
sha256sum -c SHA256SUMS
sha256sum -c apk-engine-toolkit-ubuntu-24.04-arm64.tar.xz.sha256
mkdir -p ./toolchains
tar -xJf apk-engine-toolkit-ubuntu-24.04-arm64.tar.xz -C ./toolchains
```

These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.
