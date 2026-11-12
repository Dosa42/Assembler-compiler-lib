import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import urllib.request

root = Path(os.environ['GITHUB_WORKSPACE'])
component = os.environ['COMPONENT']
source = root / 'sources' / component
work = root / 'build' / component
work.mkdir(parents=True, exist_ok=True)
logs = root / 'build-logs'
logs.mkdir(exist_ok=True)
stage = root / 'stage'
prefix = stage / 'opt' / component
prefix.mkdir(parents=True, exist_ok=True)
dist = root / 'dist'
dist.mkdir(exist_ok=True)
smoke = work / 'smoke'
smoke.mkdir(exist_ok=True)
environment = dict(os.environ)
environment['LD_LIBRARY_PATH'] = str(prefix / 'lib') + os.pathsep + environment.get('LD_LIBRARY_PATH', '')
environment['CARGO_BUILD_JOBS'] = '3'
environment['CARGO_INCREMENTAL'] = '0'
checks = []
dependencies = []

def run(label, command, cwd=None, env=None):
    path = logs / (component + '-' + label + '.log')
    print(label + ': ' + ' '.join(str(item) for item in command), flush=True)
    with path.open('w') as output:
        result = subprocess.run([str(item) for item in command], cwd=cwd or source, env=env or environment, stdout=output, stderr=subprocess.STDOUT)
    output = path.read_text(errors='replace')
    if result.returncode:
        print('\n'.join(output.splitlines()[-100:]), flush=True)
        result.check_returncode()
    return output

def cmake(extra):
    run('configure', ['cmake', '-S', source, '-B', work, '-G', 'Ninja', '-DCMAKE_BUILD_TYPE=Release', '-DCMAKE_C_COMPILER=clang', '-DCMAKE_CXX_COMPILER=clang++', '-DCMAKE_C_FLAGS_RELEASE=-O2 -DNDEBUG', '-DCMAKE_CXX_FLAGS_RELEASE=-O2 -DNDEBUG', '-DCMAKE_INSTALL_PREFIX=/opt/' + component, '-DCMAKE_INSTALL_LIBDIR=lib', '-DCMAKE_INSTALL_RPATH=$ORIGIN/../lib', *extra])
    run('compile', ['cmake', '--build', work, '--parallel', '3'])
    run('install', ['cmake', '--install', work, '--strip'], env=dict(environment, DESTDIR=str(stage)))

wat = smoke / 'answer.wat'
wat.write_text('(module (func (export "answer") (result i32) i32.const 42))\n')

if component == 'rust':
    if os.environ.get('RECOVER_STAGE') != '1':
        manifest = json.loads((root / 'sources/upstreams.json').read_text())
        upstream = next(item for item in manifest['repositories'] if item['path'] == 'sources/rust')
        url = 'https://api.github.com/repos/rust-lang/rust/git/commits/' + upstream['commit']
        with urllib.request.urlopen(url) as response:
            commit = json.load(response)
        date = commit['committer']['date'][:10]
        (source / 'git-commit-info').write_text(upstream['commit'] + '\n' + upstream['commit'][:9] + '\n' + date + '\n')
        (source / 'git-commit-hash').write_text(upstream['commit'] + '\n')
        configuration = '\n'.join([
            'profile = "dist"',
            'change-id = "ignore"',
            '[build]',
            'build-dir = ' + json.dumps(str(work / 'bootstrap')),
            'build-stage = 2',
            'install-stage = 2',
            'host = ["aarch64-unknown-linux-gnu"]',
            'target = ["aarch64-unknown-linux-gnu", "wasm32-unknown-unknown"]',
            'submodules = false',
            'docs = false',
            'extended = true',
            'tools = ["cargo", "clippy", "rustfmt"]',
            'locked-deps = true',
            '[llvm]',
            'download-ci-llvm = false',
            'ninja = true',
            'link-jobs = 1',
            'use-linker = "lld"',
            '[rust]',
            'channel = "nightly"',
            'download-rustc = false',
            'lld = true',
            'debuginfo-level = 0',
            '[install]',
            'prefix = "/opt/rust"',
            'sysconfdir = "etc"',
            '[target.aarch64-unknown-linux-gnu]',
            'cc = "clang"',
            'cxx = "clang++"',
            'linker = "clang"',
            '',
        ])
        configuration_path = work / 'bootstrap.toml'
        configuration_path.write_text(configuration)
        command = ['python3', source / 'x.py', '--config', configuration_path]
        run('compile-stage2', [*command, 'build', '--stage', '2', '-j', '3', 'compiler/rustc', 'library/std', 'src/tools/cargo', 'src/tools/clippy', 'src/tools/rustfmt'])
        run('install-stage2', [*command, 'install', '--stage', '2', '-j', '3'], env=dict(environment, DESTDIR=str(stage)))
    compiler = prefix / 'bin/rustc'
    run('rustc-version', [compiler, '--version', '--verbose'], cwd=smoke)
    native = smoke / 'native.rs'
    native.write_text('fn main() { println!("{}", 6 * 7); }\n')
    run('compile-native', [compiler, '--edition=2021', native, '-o', smoke / 'native'], cwd=smoke)
    assert run('execute-native', [smoke / 'native'], cwd=smoke).strip() == '42'
    wasm = smoke / 'answer.rs'
    wasm.write_text('#[no_mangle]\npub extern "C" fn answer() -> u32 { 42 }\n')
    run('compile-wasm', [compiler, '--edition=2021', '--crate-type=cdylib', '--target', 'wasm32-unknown-unknown', wasm, '-o', smoke / 'answer.wasm'], cwd=smoke)
    assert (smoke / 'answer.wasm').read_bytes().startswith(b'\0asm')
    project = smoke / 'cargo-check'
    (project / 'src').mkdir(parents=True)
    (project / 'Cargo.toml').write_text('[package]\nname = "native-check"\nversion = "0.1.0"\nedition = "2021"\n')
    (project / 'src/main.rs').write_text('fn main() { println!("{}", 6 * 7); }\n')
    cargo_env = dict(environment, RUSTC=str(compiler))
    assert run('cargo-native', [prefix / 'bin/cargo', 'run', '--offline', '--quiet'], cwd=project, env=cargo_env).strip() == '42'
    for tool in ['rustfmt', 'clippy-driver']:
        run(tool + '-version', [prefix / 'bin' / tool, '--version'], cwd=smoke)
    checks.extend(['Stage2 rustc compiled from imported Rust and LLVM sources', 'Native Rust program and source-built Cargo execute successfully', 'Source-built standard library and rustc produce WebAssembly', 'rustfmt and Clippy executables available'])
elif component in ['rustup', 'wasm-tools']:
    binary = 'rustup-init' if component == 'rustup' else 'wasm-tools'
    run('compile', ['cargo', '+stable', 'build', '--release', '--locked', '--manifest-path', source / 'Cargo.toml', '--bin', binary, '--target-dir', work / 'target'])
    (prefix / 'bin').mkdir(exist_ok=True)
    shutil.copy2(work / 'target/release' / binary, prefix / 'bin' / binary)
    if component == 'rustup':
        shutil.copy2(prefix / 'bin/rustup-init', prefix / 'bin/rustup')
        test_env = dict(environment, RUSTUP_HOME=str(smoke / 'rustup'), CARGO_HOME=str(smoke / 'cargo'))
        run('installer-version', [prefix / 'bin/rustup-init', '--version'], cwd=smoke, env=test_env)
        run('install-proxies', [prefix / 'bin/rustup-init', '-y', '--no-modify-path', '--default-toolchain', 'none'], cwd=smoke, env=test_env)
        run('manager-version', [smoke / 'cargo/bin/rustup', '--version'], cwd=smoke, env=test_env)
        run('toolchain-list', [smoke / 'cargo/bin/rustup', 'toolchain', 'list'], cwd=smoke, env=test_env)
        checks.append('Source-built rustup-init installs runnable rustup and compiler proxies')
    else:
        tool = prefix / 'bin/wasm-tools'
        run('version', [tool, '--version'])
        run('parse-wat', [tool, 'parse', wat, '-o', smoke / 'answer.wasm'])
        run('validate', [tool, 'validate', smoke / 'answer.wasm'])
        assert 'answer' in run('print', [tool, 'print', smoke / 'answer.wasm'])
        checks.append('Source-built wasm-tools parses, validates and prints WebAssembly')
    dependencies.append('Official stable Rust bootstrap toolchain and locked Cargo dependencies')
elif component == 'wabt':
    cmake(['-DBUILD_TESTS=OFF', '-DBUILD_TOOLS=ON'])
    run('wat-to-wasm', [prefix / 'bin/wat2wasm', wat, '-o', smoke / 'answer.wasm'])
    run('validate', [prefix / 'bin/wasm-validate', smoke / 'answer.wasm'])
    result = run('interpret', [prefix / 'bin/wasm-interp', smoke / 'answer.wasm', '--run-all-exports'])
    assert 'i32:42' in result
    checks.append('Source-built WABT assembles, validates and interprets answer() = 42')
elif component == 'binaryen':
    cmake(['-DBUILD_TESTS=OFF', '-DBUILD_TOOLS=ON', '-DINSTALL_LIBS=ON'])
    run('version', [prefix / 'bin/wasm-opt', '--version'])
    run('optimize', [prefix / 'bin/wasm-opt', wat, '-O2', '-o', smoke / 'answer.wasm'])
    assert 'answer' in run('disassemble', [prefix / 'bin/wasm-dis', smoke / 'answer.wasm'])
    checks.append('Source-built Binaryen optimizes and disassembles WebAssembly')
elif component == 'wasi-sdk':
    manifest = json.loads((root / 'sources/upstreams.json').read_text())
    upstream = next(item for item in manifest['repositories'] if item['path'] == 'sources/wasi-sdk')
    llvm = next(item for item in upstream['submodules'] if item['path'] == 'src/llvm-project')
    for label, path, url, commit, tags in [
        ('sdk', source, upstream['url'], upstream['commit'], True),
        ('llvm', source / 'src/llvm-project', llvm['url'], llvm['commit'], False),
    ]:
        if not (path / '.git').exists():
            run(label + '-git-init', ['git', 'init', path], cwd=path)
            options = ['--tags'] if tags else ['--no-tags', '--depth=1']
            run(label + '-git-fetch', ['git', 'fetch', *options, url, commit], cwd=path)
            run(label + '-git-index', ['git', 'reset', '--mixed', commit], cwd=path)
    run('sdk-version', ['python3', source / 'version.py'], cwd=source)
    toolchain = work / 'toolchain'
    sysroot = work / 'sysroot'
    flags = '-DLLVM_PARALLEL_LINK_JOBS=1 -DCMAKE_INSTALL_RPATH=$ORIGIN/../lib'
    run('configure-toolchain', ['cmake', '-S', source, '-B', toolchain, '-G', 'Ninja', '-DCMAKE_BUILD_TYPE=Release', '-DCMAKE_C_COMPILER=clang', '-DCMAKE_CXX_COMPILER=clang++', '-DWASI_SDK_BUILD_TOOLCHAIN=ON', '-DLLVM_CMAKE_FLAGS=' + flags, '-DCMAKE_INSTALL_PREFIX=' + str(prefix)])
    run('compile-toolchain', ['cmake', '--build', toolchain, '--target', 'install', '--parallel', '3'])
    run('configure-sysroot', ['cmake', '-S', source, '-B', sysroot, '-G', 'Ninja', '-DCMAKE_INSTALL_PREFIX=' + str(prefix), '-DCMAKE_TOOLCHAIN_FILE=' + str(prefix / 'share/cmake/wasi-sdk-p2.cmake'), '-DCMAKE_C_COMPILER_WORKS=ON', '-DCMAKE_CXX_COMPILER_WORKS=ON', '-DWASI_SDK_INSTALL_TO_CLANG_RESOURCE_DIR=ON'])
    run('compile-sysroot', ['cmake', '--build', sysroot, '--target', 'install', '--parallel', '3'])
    hello = smoke / 'hello.c'
    hello.write_text('#include <stdio.h>\nint main(void) { puts("WASI ARM64 compiler OK"); return 0; }\n')
    run('version', [prefix / 'bin/clang', '--version'])
    run('compile-wasi', [prefix / 'bin/clang', '--target=wasm32-wasip1', '--sysroot=' + str(prefix / 'share/wasi-sysroot'), hello, '-o', smoke / 'hello.wasm'])
    assert (smoke / 'hello.wasm').read_bytes().startswith(b'\0asm')
    checks.append('Source-built WASI Clang and sysroot compile a C program to WebAssembly')
    dependencies.append('Upstream WASI SDK build dependencies, patches and wasm-component-ld crate')
elif component == 'emsdk':
    shutil.copytree(source, prefix, symlinks=True, dirs_exist_ok=True)
    assert 'install' in run('manager-help', ['python3', prefix / 'emsdk.py', 'help'], cwd=prefix)
    checks.append('Imported emsdk Python manager packaged and its command interface executed')
elif component == 'emscripten':
    sdk = root / 'sources/emsdk'
    shutil.copytree(sdk, prefix, symlinks=True, dirs_exist_ok=True)
    version = (source / 'test/emsdk_version.txt').read_text().strip()
    run('install-sdk-dependencies', ['python3', prefix / 'emsdk.py', 'install', version], cwd=prefix)
    run('activate-sdk-dependencies', ['python3', prefix / 'emsdk.py', 'activate', version], cwd=prefix)
    frontend = prefix / 'upstream/emscripten'
    shutil.rmtree(frontend)
    shutil.copytree(source, frontend, symlinks=True)
    node = next(path for path in (prefix / 'node').glob('*/bin/node') if path.is_file())
    env = dict(environment, EM_CONFIG=str(prefix / '.emscripten'), PATH=str(node.parent) + os.pathsep + environment['PATH'])
    run('npm-packages', ['npm', 'ci'], cwd=frontend, env=env)
    run('entry-points', ['python3', frontend / 'tools/maint/create_entry_points.py'], cwd=frontend, env=env)
    output = frontend / 'out'
    output.mkdir(exist_ok=True)
    for action in ['npm packages', 'create entry points', 'git submodules', 'install hooks']:
        (output / (action.replace(' ', '_') + '.stamp')).touch()
    relative_node = str(node.relative_to(prefix))
    configuration = '\n'.join(['import os', 'ROOT = os.path.dirname(os.path.abspath(__file__))', 'LLVM_ROOT = os.path.join(ROOT, "upstream", "bin")', 'BINARYEN_ROOT = os.path.join(ROOT, "upstream")', 'NODE_JS = [os.path.join(ROOT, ' + json.dumps(relative_node) + ')]', 'CACHE = os.path.join(ROOT, "upstream", "emscripten", "cache")', ''])
    (prefix / '.emscripten').write_text(configuration)
    hello = smoke / 'hello.c'
    hello.write_text('#include <stdio.h>\nint main(void) { puts("Emscripten WebAssembly OK"); return 0; }\n')
    compiler = frontend / 'emcc.py'
    run('compiler-version', ['python3', compiler, '--version'], cwd=smoke, env=env)
    run('compile-c-to-wasm', ['python3', compiler, hello, '-O2', '-sENVIRONMENT=node', '-o', smoke / 'hello.js'], cwd=smoke, env=env)
    assert 'Emscripten WebAssembly OK' in run('execute-wasm-node', [node, smoke / 'hello.js'], cwd=smoke, env=env)
    checks.append('Imported Emscripten frontend compiles C to WebAssembly and executes it with Node.js')
    dependencies.append('Official emsdk LLVM, Binaryen and Node dependencies pinned by Emscripten test/emsdk_version.txt: ' + version)
else:
    raise ValueError(component)

metadata = prefix / 'share/build-info'
metadata.mkdir(parents=True, exist_ok=True)
shutil.copy2(root / 'sources/upstreams.json', metadata / 'upstreams.json')
report = {'component': component, 'platform': 'ubuntu-24.04-arm64', 'repository_commit': os.environ['GITHUB_SHA'], 'checks': checks, 'dependencies': dependencies, 'completed_at': datetime.datetime.now(datetime.timezone.utc).isoformat()}
(metadata / 'build.json').write_text(json.dumps(report, indent=2) + '\n')
(metadata / 'SOURCE.txt').write_text('https://github.com/' + os.environ['GITHUB_REPOSITORY'] + '/tree/' + os.environ['GITHUB_SHA'] + '/sources/' + component + '\n')
for pattern in ['LICENSE*', 'COPYING*']:
    for path in source.glob(pattern):
        if path.is_file():
            licenses = metadata / 'licenses'
            licenses.mkdir(exist_ok=True)
            shutil.copy2(path, licenses / path.name)
archive = dist / (component + '-ubuntu-24.04-arm64.tar.xz')
run('package', ['tar', '--sort=name', '--owner=0', '--group=0', '--numeric-owner', '-cJf', archive, '-C', stage, '.'], cwd=work, env=dict(environment, XZ_OPT='-T2 -1'))
digest = hashlib.sha256()
with archive.open('rb') as stream:
    for chunk in iter(lambda: stream.read(1048576), b''):
        digest.update(chunk)
archive.with_suffix(archive.suffix + '.sha256').write_text(digest.hexdigest() + '  ' + archive.name + '\n')
with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as summary:
    summary.write(component + ' built, packaged and verified\n\n' + '\n'.join('- ' + item for item in checks) + '\n')
