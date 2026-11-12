import argparse
import hashlib
import json
from pathlib import Path
import shutil
import tarfile
import urllib.request

parser = argparse.ArgumentParser(description='Download and restore a source-built ARM64 distribution from all-dist.')
parser.add_argument('component')
parser.add_argument('--output', type=Path, default=Path('downloads'))
parser.add_argument('--extract', type=Path)
arguments = parser.parse_args()
repository = 'Dosa42/Assembler-compiler-lib'
platform = 'ubuntu-24.04-arm64'
reference = json.load(urllib.request.urlopen('https://api.github.com/repos/' + repository + '/git/ref/heads/main'))['object']['sha']
base = 'https://raw.githubusercontent.com/' + repository + '/' + reference + '/all-dist/' + platform + '/' + arguments.component + '/'
manifest = json.load(urllib.request.urlopen(base + 'manifest.json'))
arguments.output.mkdir(parents=True, exist_ok=True)

def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1048576), b''):
            value.update(chunk)
    return value.hexdigest()

parts = []
for item in manifest['files']:
    path = arguments.output / item['name']
    if not path.is_file() or digest(path) != item['sha256']:
        print('Downloading ' + item['name'], flush=True)
        with urllib.request.urlopen(base + item['name']) as source, path.open('wb') as output:
            shutil.copyfileobj(source, output)
    if digest(path) != item['sha256']:
        raise RuntimeError('Checksum mismatch: ' + path.name)
    parts.append(path)
archive = arguments.output / manifest['archive']['name']
if len(parts) > 1:
    with archive.open('wb') as output:
        for part in parts:
            with part.open('rb') as source:
                shutil.copyfileobj(source, output)
if digest(archive) != manifest['archive']['sha256']:
    raise RuntimeError('Restored archive checksum mismatch')
print('Verified archive: ' + str(archive.resolve()), flush=True)
if arguments.extract:
    arguments.extract.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, 'r:xz') as source:
        source.extractall(arguments.extract, filter='data')
    print('Extracted installation: ' + str(arguments.extract.resolve()), flush=True)
