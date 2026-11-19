import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
import tempfile
import zipfile


root = pathlib.Path(__file__).resolve().parent
parser = argparse.ArgumentParser()
parser.add_argument('--library', type=pathlib.Path, default=root.parent.parent / 'engine_library')
args = parser.parse_args()
library = args.library.resolve()
engines = ['zip4j', 'zipalign-java', 'smali', 'apksig', 'axml', 'rsm-process-parser', 'rsm-meminfo-parser']
jars = []
for name in engines:
    found = sorted((library / name / 'lib').glob('*.jar'))
    if not found:
        raise SystemExit('Missing engine artifacts: ' + name)
    jars.extend(found)
jars.extend(sorted((root / 'vendor').glob('*.jar')))
if not any(p.name.startswith('gson-') and not p.name.endswith('-sources.jar') for p in jars):
    raise SystemExit('Missing Gson artifact')
sources = sorted((root / 'src' / 'main' / 'java').rglob('*.java'))
brokkr_schema = library / 'brokkr/schemas/result.schema.json'
json.loads(brokkr_schema.read_text())
dist = root / 'dist'
dist.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix='.build-', dir=root) as scratch:
    classes = pathlib.Path(scratch) / 'classes'
    classes.mkdir()
    command = ['javac', '-J-Xmx256m', '-J-XX:ActiveProcessorCount=2', '--release', '17', '-encoding', 'UTF-8', '-cp', ':'.join(map(str, jars)), '-d', str(classes), *map(str, sources)]
    subprocess.run(command, check=True)
    output = pathlib.Path(scratch) / 'apk-engine.jar'
    seen = {}
    services = {}
    with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED) as out:
        out.writestr('META-INF/MANIFEST.MF', 'Manifest-Version: 1.0\nMain-Class: dev.ayrvokan.apkengine.Main\nImplementation-Version: 0.3.0\n\n')
        out.writestr('brokkr-result.schema.json', brokkr_schema.read_bytes())
        for path in sorted(classes.rglob('*')):
            if path.is_file():
                relative = path.relative_to(classes).as_posix()
                data = path.read_bytes()
                out.writestr(relative, data)
                seen[relative] = hashlib.sha256(data).hexdigest()
        for jar in jars:
            with zipfile.ZipFile(jar) as source:
                for item in source.infolist():
                    name = item.filename
                    upper = name.upper()
                    if item.is_dir() or upper == 'META-INF/MANIFEST.MF' or upper.endswith(('.SF', '.RSA', '.DSA')) or name == 'module-info.class' or name.startswith('META-INF/versions/'):
                        continue
                    data = source.read(item)
                    if name.startswith('META-INF/services/'):
                        services.setdefault(name, set()).update(line.strip() for line in data.decode().splitlines() if line.strip())
                        continue
                    digest = hashlib.sha256(data).hexdigest()
                    if name in seen:
                        if name.endswith('.class') and seen[name] != digest:
                            raise SystemExit('Conflicting class: ' + name + ' in ' + str(jar))
                        continue
                    seen[name] = digest
                    out.writestr(name, data)
        for name, lines in sorted(services.items()):
            out.writestr(name, '\n'.join(sorted(lines)) + '\n')
    shutil.copy2(output, dist / output.name)
manifest = {
    'schema': 'apk-engine.build/1',
    'version': '0.3.0',
    'target': 'JVM intermediate for native AOT build',
    'java_release': 17,
    'sources': {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources},
    'dependencies': [{'path': str(p), 'sha256': hashlib.sha256(p.read_bytes()).hexdigest(), 'bytes': p.stat().st_size} for p in jars],
    'native_result_schema': {'path': str(brokkr_schema), 'sha256': hashlib.sha256(brokkr_schema.read_bytes()).hexdigest()},
    'artifact': {'path': str(dist / 'apk-engine.jar'), 'sha256': hashlib.sha256((dist / 'apk-engine.jar').read_bytes()).hexdigest(), 'bytes': (dist / 'apk-engine.jar').stat().st_size},
}
(dist / 'build.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps(manifest['artifact']))
