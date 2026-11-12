import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import zipfile

root = Path(os.environ['GITHUB_WORKSPACE'])
scratch = Path(os.environ['RUNNER_TEMP']) / 'publish-all-dist'
scratch.mkdir(parents=True, exist_ok=True)
repository = os.environ['GITHUB_REPOSITORY']
branch = os.environ.get('DESTINATION_BRANCH', 'main')
requested = os.environ.get('REQUESTED_COMPONENT', 'all')
requested_run = os.environ.get('SOURCE_RUN_ID', '')
platform = 'ubuntu-24.04-arm64'
limit = 95 * 1024 * 1024
dist = root / 'all-dist'
dist.mkdir(exist_ok=True)
index_path = dist / 'index.json'
catalog = json.loads(index_path.read_text()) if index_path.exists() else {'platform': platform, 'components': {}}
fence = chr(96) * 3

def git(*arguments):
    return subprocess.check_output(['git', *arguments], cwd=root, text=True).strip()

def api(path):
    return json.loads(subprocess.check_output(['gh', 'api', path], text=True))

def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1048576), b''):
            value.update(chunk)
    return value.hexdigest()

def write_catalog():
    index_path.write_text(json.dumps(catalog, indent=2) + '\n')
    lines = ['# Built distributions', '', 'Source-built installation archives committed to this repository. Each archive contains the executables, libraries and build information produced by its verified GitHub build.', '', '| Component | Platform | Build run | Files |', '| --- | --- | --- | --- |']
    for name, record in sorted(catalog['components'].items()):
        relative = platform + '/' + name
        files = ', '.join('[' + item['name'] + '](' + relative + '/' + item['name'] + ')' for item in record['files'])
        lines.append('| [' + name + '](' + relative + '/README.md) | ' + platform + ' | [' + str(record['run_id']) + '](' + record['run_url'] + ') | ' + files + ' |')
    lines.extend(['', '## Download and restore', '', 'Run the helper with Python 3. It downloads a component, verifies each SHA-256 checksum and restores its original installation archive. Add --extract to unpack the installation tree.', '', fence + 'sh', 'curl -fL https://raw.githubusercontent.com/' + repository + '/main/all-dist/restore.py -o restore.py', 'python3 restore.py gcc --output ./downloads --extract ./toolchains', 'python3 restore.py zig --output ./downloads --extract ./toolchains', fence, '', 'You can also download the files directly using the links above. Archives larger than 95 MiB are stored in numbered parts. Each component directory contains its SHA-256 checksums, provenance and reconstruction instructions.'])
    (dist / 'README.md').write_text('\n'.join(lines) + '\n')

def push():
    for attempt in range(4):
        result = subprocess.run(['git', '-c', 'pack.threads=2', '-c', 'pack.windowMemory=128m', 'push', 'origin', 'HEAD:refs/heads/' + branch], cwd=root)
        if result.returncode == 0:
            return
        if attempt == 3:
            result.check_returncode()
        subprocess.run(['git', 'fetch', 'origin', branch], cwd=root, check=True)
        subprocess.run(['git', 'rebase', 'origin/' + branch], cwd=root, check=True)

route = 'repos/' + repository + '/actions/'
if requested_run:
    int(requested_run)
    route += 'runs/' + requested_run + '/artifacts'
else:
    route += 'artifacts'
selected = {}
page = 1
while True:
    records = api(route + '?per_page=100&page=' + str(page))['artifacts']
    for artifact in records:
        match = re.fullmatch(r'([a-z0-9][a-z0-9-]*)-ubuntu-24\.04-arm64', artifact['name'])
        if artifact['expired'] or match is None:
            continue
        if artifact.get('workflow_run', {}).get('head_branch', branch) != branch:
            continue
        component = match.group(1)
        if requested not in ['all', component]:
            continue
        previous = selected.get(component)
        if previous is None or (artifact['created_at'], artifact['id']) > (previous['created_at'], previous['id']):
            selected[component] = artifact
    if len(records) < 100:
        break
    page += 1
if requested != 'all' and requested not in selected:
    raise RuntimeError('No downloadable verified build artifact found for ' + requested)

git('config', 'user.name', 'github-actions[bot]')
git('config', 'user.email', '41898282+github-actions[bot]@users.noreply.github.com')
published = []
for component, artifact in sorted(selected.items()):
    previous = catalog['components'].get(component)
    destination = dist / platform / component
    if previous and previous['artifact_created_at'] > artifact['created_at']:
        print('Retaining newer distribution for ' + component, flush=True)
        continue
    if previous and previous['artifact_id'] == artifact['id']:
        if all((destination / item['name']).is_file() and digest(destination / item['name']) == item['sha256'] for item in previous['files']):
            print(component + ' is already stored and verified in all-dist', flush=True)
            continue
    package = scratch / component
    package.mkdir(exist_ok=True)
    download = package / 'artifact.zip'
    print('Downloading verified build artifact ' + artifact['name'], flush=True)
    with download.open('wb') as output:
        subprocess.run(['gh', 'api', 'repos/' + repository + '/actions/artifacts/' + str(artifact['id']) + '/zip'], stdout=output, check=True)
    if artifact.get('digest'):
        algorithm, expected = artifact['digest'].split(':', 1)
        if algorithm != 'sha256' or digest(download) != expected:
            raise RuntimeError('Artifact ZIP checksum mismatch: ' + component)
    archive_name = artifact['name'] + '.tar.xz'
    archive = package / archive_name
    with zipfile.ZipFile(download) as source:
        names = {Path(name).name: name for name in source.namelist() if not name.endswith('/')}
        if archive_name not in names or archive_name + '.sha256' not in names:
            raise RuntimeError('Installation archive or checksum missing: ' + component)
        expected = source.read(names[archive_name + '.sha256']).decode().split()[0]
        with source.open(names[archive_name]) as stream, archive.open('wb') as output:
            shutil.copyfileobj(stream, output)
    actual = digest(archive)
    if actual != expected:
        raise RuntimeError('Installation archive checksum mismatch: ' + component)
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    files = []
    if archive.stat().st_size <= limit:
        target = destination / archive_name
        shutil.copy2(archive, target)
        files.append({'name': target.name, 'size': target.stat().st_size, 'sha256': actual})
    else:
        with archive.open('rb') as source:
            number = 1
            while True:
                chunk = source.read(limit)
                if not chunk:
                    break
                target = destination / (archive_name + '.part' + str(number).zfill(4))
                target.write_bytes(chunk)
                files.append({'name': target.name, 'size': len(chunk), 'sha256': hashlib.sha256(chunk).hexdigest()})
                number += 1
    run = artifact['workflow_run']
    record = {'component': component, 'platform': platform, 'artifact_id': artifact['id'], 'artifact_created_at': artifact['created_at'], 'artifact_digest': artifact.get('digest'), 'run_id': run['id'], 'run_url': 'https://github.com/' + repository + '/actions/runs/' + str(run['id']), 'source_commit': run['head_sha'], 'archive': {'name': archive_name, 'size': archive.stat().st_size, 'sha256': actual}, 'files': files, 'stored_at': datetime.datetime.now(datetime.timezone.utc).isoformat()}
    (destination / 'manifest.json').write_text(json.dumps(record, indent=2) + '\n')
    (destination / 'SHA256SUMS').write_text(''.join(item['sha256'] + '  ' + item['name'] + '\n' for item in files))
    (destination / (archive_name + '.sha256')).write_text(actual + '  ' + archive_name + '\n')
    instructions = ['# ' + component + ' for ' + platform, '', 'The installation archive contains this component\'s source-built executables, libraries and build provenance.', '', '[Original build run](' + record['run_url'] + ')', '', 'Source repository commit: ' + record['source_commit'] + '.', '', '## Files', '']
    instructions.extend('- [' + item['name'] + '](' + item['name'] + ') (' + str(item['size']) + ' bytes)' for item in files)
    instructions.extend(['', 'Download the files above, SHA256SUMS and ' + archive_name + '.sha256 into one directory.', '', fence + 'sh', 'sha256sum -c SHA256SUMS'])
    if len(files) > 1:
        instructions.append('cat ' + archive_name + '.part* > ' + archive_name)
    instructions.extend(['sha256sum -c ' + archive_name + '.sha256', 'mkdir -p ./toolchains', 'tar -xJf ' + archive_name + ' -C ./toolchains', fence, '', 'These outputs run on ARM64 GNU/Linux. Newlib provides bare-metal target libraries; Glibc provides a separate sysroot.'])
    (destination / 'README.md').write_text('\n'.join(instructions) + '\n')
    catalog['components'][component] = record
    write_catalog()
    git('add', '--force', '--', 'all-dist')
    if subprocess.run(['git', 'diff', '--cached', '--quiet'], cwd=root).returncode != 0:
        git('commit', '--quiet', '-m', 'Store verified ' + component + ' executables and libraries in all-dist')
        push()
    published.append(component)
    shutil.rmtree(package)
    print('Committed ' + component + ' distribution to ' + str(destination.relative_to(root)), flush=True)
with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as summary:
    summary.write('Distributions stored in repository: ' + (', '.join(published) or 'Already current') + '\n')
