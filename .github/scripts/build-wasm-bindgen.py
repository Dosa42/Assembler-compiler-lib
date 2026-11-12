import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

root = Path(os.environ['GITHUB_WORKSPACE'])
work = root / '.build/wasm-bindgen'
work.mkdir(parents=True, exist_ok=True)
prefix = work / 'stage/opt/wasm-bindgen'
rust = work / 'toolchain/opt/rust'
version = '0.2.130'
subprocess.run(['python3', root / 'all-dist/restore.py', 'rust', '--output', work / 'downloads', '--extract', work / 'toolchain'], check=True)
environment = dict(os.environ, RUSTC=str(rust / 'bin/rustc'), PATH=str(rust / 'bin') + os.pathsep + os.environ['PATH'], LD_LIBRARY_PATH=str(rust / 'lib') + os.pathsep + os.environ.get('LD_LIBRARY_PATH', ''), CARGO_BUILD_JOBS='3')
subprocess.run([rust / 'bin/cargo', 'install', 'wasm-bindgen-cli', '--version', version, '--locked', '--root', prefix], env=environment, check=True)
compiler = subprocess.check_output([rust / 'bin/rustc', '--version', '--verbose'], env=environment, text=True).strip()
actual = subprocess.check_output([prefix / 'bin/wasm-bindgen', '--version'], env=environment, text=True).strip()
if actual != 'wasm-bindgen ' + version:
    raise RuntimeError(actual)
smoke = work / 'smoke'
(smoke / 'src').mkdir(parents=True, exist_ok=True)
(smoke / 'Cargo.toml').write_text('[package]\nname="browser-check"\nversion="0.1.0"\nedition="2021"\n[lib]\ncrate-type=["cdylib"]\n[dependencies]\nwasm-bindgen="=' + version + '"\n')
(smoke / 'src/lib.rs').write_text('use wasm_bindgen::prelude::*;\n#[wasm_bindgen]\npub fn answer() -> u32 { 42 }\n')
subprocess.run([rust / 'bin/cargo', 'build', '--manifest-path', smoke / 'Cargo.toml', '--release', '--target', 'wasm32-unknown-unknown'], env=environment, check=True)
wasm = smoke / 'target/wasm32-unknown-unknown/release/browser_check.wasm'
for target in ['web', 'nodejs']:
    subprocess.run([prefix / 'bin/wasm-bindgen', '--target', target, '--out-dir', smoke / target, wasm], env=environment, check=True)
subprocess.run(['node', '-e', "const wasm=require(process.argv[1]); if(wasm.answer()!==42)process.exit(1)", str(smoke / 'nodejs/browser_check.js')], check=True)
metadata = prefix / 'share/build-info'
metadata.mkdir(parents=True)
report = {'component': 'wasm-bindgen', 'version': version, 'platform': 'ubuntu-24.04-arm64', 'repository_commit': os.environ['GITHUB_SHA'], 'rustc': compiler, 'source': 'https://crates.io/crates/wasm-bindgen-cli/' + version, 'checks': ['source-built ARM64 CLI executes', 'source-built Rust toolchain compiles wasm32 browser library', 'web bindings generated', 'Node.js executes bound WASM answer() = 42']}
(metadata / 'build.json').write_text(json.dumps(report, indent=2) + '\n')
shutil.copy2(smoke / 'Cargo.lock', metadata / 'smoke.Cargo.lock')
dist = root / 'dist'
dist.mkdir(exist_ok=True)
archive = dist / 'wasm-bindgen-ubuntu-24.04-arm64.tar.xz'
subprocess.run(['tar', '--sort=name', '--owner=0', '--group=0', '--numeric-owner', '-cJf', archive, '-C', work / 'stage', '.'], check=True)
with archive.open('rb') as source:
    digest = hashlib.file_digest(source, 'sha256').hexdigest()
archive.with_suffix('.xz.sha256').write_text(digest + '  ' + archive.name + '\n')
with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as output:
    output.write('wasm-bindgen ' + version + ' built using the Assembler-compiler-lib Rust distribution; web output generated and WASM executed through Node.js.\n')
