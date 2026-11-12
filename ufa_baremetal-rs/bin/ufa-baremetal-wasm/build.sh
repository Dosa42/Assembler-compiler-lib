#!/usr/bin/env bash
# Build all three WebAssembly modules and embed them in one offline page.
set -euo pipefail
caller_dir="$PWD"
cd "$(dirname "${BASH_SOURCE[0]}")"

target=wasm32-unknown-unknown
target_dir="${CARGO_TARGET_DIR:-$PWD/target}"
[[ "$target_dir" = /* ]] || target_dir="$caller_dir/$target_dir"

for manifest in Cargo.toml ../uagl-assembler-wasm/Cargo.toml ../../c-uagl-rust/Cargo.toml; do
    cargo build --locked --release --lib --target "$target" --target-dir "$target_dir" \
        --manifest-path "$manifest"
done

release_dir="$target_dir/$target/release"
python3 - "$release_dir" <<'PY'
import base64
from pathlib import Path
from shutil import copyfile
import sys

web = Path("www")
release = Path(sys.argv[1])
page = (web / "template.html.in").read_text(encoding="utf-8")

def embed(marker, content):
    global page
    if page.count(marker) != 1:
        raise SystemExit(f"Expected one {marker} in the template")
    page = page.replace(marker, content)

modules = {
    "__WASM_BASE64__": "ufa_baremetal_wasm.wasm",
    "__ASSEMBLER_WASM_BASE64__": "uagl_assembler_wasm.wasm",
    "__CONVERTER_WASM_BASE64__": "c_uagl_converter.wasm",
}
for marker, filename in modules.items():
    path = release / filename
    data = path.read_bytes()
    if not data.startswith(b"\0asm\x01\0\0\0"):
        raise SystemExit(f"Invalid WebAssembly module: {path}")
    embed(marker, base64.b64encode(data).decode("ascii"))

scripts = {
    "__PIPELINE_CORE_JS__": "pipeline-core.js",
    "__WASM_TOOLS_JS__": "wasm-tools.js",
    "__UNIFIED_UI_JS__": "unified-ui.js",
}
for marker, filename in scripts.items():
    source = (web / filename).read_text(encoding="utf-8")
    if "</script" in source.lower():
        raise SystemExit(f"Unexpected script closing tag in {filename}")
    embed(marker, source)

page_bytes = page.encode("utf-8")
output = Path("../../ufa-baremetal-playground.html")
output.write_bytes(page_bytes)
print(f"Wrote {output} ({len(page_bytes)} bytes)")

for output in (
    Path("../../dist/wasm/uagl_assembler_wasm.wasm"),
    Path("../../c-uagl-rust/dist/wasm/c_uagl_converter.wasm"),
):
    output.parent.mkdir(parents=True, exist_ok=True)
    copyfile(release / output.name, output)
    print(f"Wrote {output}")
PY
