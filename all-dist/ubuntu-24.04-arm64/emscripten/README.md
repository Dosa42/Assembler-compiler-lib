# emscripten for ubuntu-24.04-arm64

The installation archive contains this component's source-built executables, libraries and build provenance.

[Original build run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43179370823)

Source repository commit: 24ea2a565e23022343bb4ed7de2a75d4341064b6.

## Files

- [emscripten-ubuntu-24.04-arm64.tar.xz.part0001](emscripten-ubuntu-24.04-arm64.tar.xz.part0001) (99614720 bytes)
- [emscripten-ubuntu-24.04-arm64.tar.xz.part0002](emscripten-ubuntu-24.04-arm64.tar.xz.part0002) (99614720 bytes)
- [emscripten-ubuntu-24.04-arm64.tar.xz.part0003](emscripten-ubuntu-24.04-arm64.tar.xz.part0003) (71844440 bytes)

Download the files above, SHA256SUMS and emscripten-ubuntu-24.04-arm64.tar.xz.sha256 into one directory.

```sh
sha256sum -c SHA256SUMS
cat emscripten-ubuntu-24.04-arm64.tar.xz.part* > emscripten-ubuntu-24.04-arm64.tar.xz
sha256sum -c emscripten-ubuntu-24.04-arm64.tar.xz.sha256
mkdir -p ./toolchains
tar -xJf emscripten-ubuntu-24.04-arm64.tar.xz -C ./toolchains
```

These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.
