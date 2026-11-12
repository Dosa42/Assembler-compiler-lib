#!/usr/bin/env bash
# Activate this bundle in the current Bash shell.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    printf 'Run source %q from Bash.\n' "$0" >&2
    exit 1
fi

rust_wasm_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P) || return 1
export RUSTUP_HOME="$rust_wasm_dir/rustup"
export CARGO_HOME="$rust_wasm_dir/cargo"
export PATH="$CARGO_HOME/bin:$PATH"
unset rust_wasm_dir
