import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import zipfile

root = Path(os.environ['GITHUB_WORKSPACE'])
scratch = Path(os.environ['RUNNER_TEMP']) / 'rust-recovery'
scratch.mkdir(exist_ok=True)
stage = root / 'stage'
stage.mkdir(exist_ok=True)
download = scratch / 'rust-stage.zip'
with download.open('wb') as output:
    subprocess.run(['gh', 'api', 'repos/' + os.environ['GITHUB_REPOSITORY'] + '/actions/artifacts/13930097735/zip'], stdout=output, check=True)
digest = hashlib.sha256()
with download.open('rb') as source:
    for chunk in iter(lambda: source.read(1048576), b''):
        digest.update(chunk)
if digest.hexdigest() != 'a5c867d7cd6b4e787523270f70be0357e4519ff414662ab33fea21b09345ef99':
    raise RuntimeError('Saved Rust stage checksum mismatch')
with zipfile.ZipFile(download) as archive:
    archive.extractall(stage)
prefix = stage / 'opt/rust'
if not (prefix / 'bin/rustc').is_file():
    raise RuntimeError('Saved source-built Rust compiler missing from expected installation tree')
for path in prefix.rglob('*'):
    if path.is_file():
        with path.open('rb') as source:
            elf = source.read(4) == b'\x7fELF'
        if elf or path.parent == prefix / 'bin':
            path.chmod(path.stat().st_mode | 0o111)
subprocess.run(['python3', root / 'all-dist/restore.py', 'llvm', '--output', scratch / 'downloads', '--extract', scratch / 'llvm'], check=True)
llvm = scratch / 'llvm/opt/llvm'
original = llvm / 'bin/lld'
host = 'aarch64-unknown-linux-gnu'
bin_directory = prefix / 'lib/rustlib' / host / 'bin'
bin_directory.mkdir(parents=True, exist_ok=True)
linker = bin_directory / 'rust-lld'
shutil.copy2(original.resolve(), linker)
library_directory = bin_directory / 'lib'
library_directory.mkdir(exist_ok=True)
dependencies = subprocess.check_output(['ldd', original], text=True)
base = {'libc.so.6', 'libm.so.6', 'libpthread.so.0', 'libdl.so.2', 'librt.so.1', 'libstdc++.so.6', 'libgcc_s.so.1'}
bundled = []
for line in dependencies.splitlines():
    match = re.match(r'\s*(\S+)\s+=>\s+(/\S+)', line)
    if match and match.group(1) not in base:
        name, source = match.groups()
        destination = library_directory / name
        shutil.copy2(Path(source).resolve(), destination)
        subprocess.run(['patchelf', '--set-rpath', '$ORIGIN', destination], check=True)
        bundled.append(name)
subprocess.run(['patchelf', '--set-rpath', '$ORIGIN/lib', linker], check=True)
subprocess.run([linker, '-flavor', 'wasm', '--version'], check=True)
metadata = prefix / 'share/build-info'
metadata.mkdir(parents=True, exist_ok=True)
report = {'source_build_run': 43179370823, 'saved_stage_artifact': 13930097735, 'saved_stage_sha256': digest.hexdigest(), 'recovery': 'Preserved source-built Rust stage2 compiler; added source-built LLD from verified LLVM distribution', 'linker_build_run': 43168282553, 'linker_source_component': 'llvm', 'linker_version': '24', 'bundled_linker_libraries': bundled}
(metadata / 'recovery.json').write_text(json.dumps(report, indent=2) + '\n')
print('Recovered source-built Rust compiler and bundled source-built LLD with relative runtime paths', flush=True)
