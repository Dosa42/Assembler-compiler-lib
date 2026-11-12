# Rust en WebAssembly voor Linux x86_64

Deze ZIP bevat rustup, de geïnstalleerde stabiele Rust-toolchain (rustc, Cargo en de standaardbibliotheek) en het WebAssembly-target `wasm32-unknown-unknown`. Gebruik op Linux x86_64 met glibc, bijvoorbeeld onder WSL op een x86_64-machine.

Pak de ZIP uit met `unzip` op Linux, zodat uitvoerrechten en symbolische links behouden blijven. Activeer de toolchain in Bash.

```bash
unzip rust-wasm-linux-x86_64.zip
source rust-wasm/activate.sh
rustc --version
cargo --version
rustup target list --installed
```

Bouw vanuit de map van je Rust-project voor WebAssembly.

```bash
cargo build --offline --target wasm32-unknown-unknown --release
```

Projecten met externe Cargo-dependencies hebben ook hun eigen dependencycache nodig voor offline builds. De bestanden in de map `rust-wasm` kunnen samen naar een andere locatie worden verplaatst.
