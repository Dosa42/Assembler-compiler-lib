# rust for ubuntu-24.04-arm64

The installation archive contains this component's source-built executables, libraries and build provenance.

[Original build run](https://github.com/Dosa42/Assembler-compiler-lib/actions/runs/43206784386)

Source repository commit: 6b829d52891963f122bec845342910cbfbc9cbe3.

## Files

- [rust-ubuntu-24.04-arm64.tar.xz.part0001](rust-ubuntu-24.04-arm64.tar.xz.part0001) (99614720 bytes)
- [rust-ubuntu-24.04-arm64.tar.xz.part0002](rust-ubuntu-24.04-arm64.tar.xz.part0002) (99614720 bytes)
- [rust-ubuntu-24.04-arm64.tar.xz.part0003](rust-ubuntu-24.04-arm64.tar.xz.part0003) (75246900 bytes)

Download the files above, SHA256SUMS and rust-ubuntu-24.04-arm64.tar.xz.sha256 into one directory.

```sh
sha256sum -c SHA256SUMS
cat rust-ubuntu-24.04-arm64.tar.xz.part* > rust-ubuntu-24.04-arm64.tar.xz
sha256sum -c rust-ubuntu-24.04-arm64.tar.xz.sha256
mkdir -p ./toolchains
tar -xJf rust-ubuntu-24.04-arm64.tar.xz -C ./toolchains
```

These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.
