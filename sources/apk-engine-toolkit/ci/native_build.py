import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / "evidence"
COMMANDS = []


def run(command, log_name, env):
    started = time.time()
    with (EVIDENCE / log_name).open("w") as log:
        process = subprocess.Popen([str(value) for value in command], cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        for line in process.stdout:
            print(line, end="", flush=True)
            log.write(line)
        code = process.wait()
    COMMANDS.append({"command": [str(value) for value in command], "exit_code": code, "seconds": round(time.time() - started, 3), "log": log_name})
    if code:
        raise RuntimeError("Command failed with exit code " + str(code) + ": " + str(command[0]))


def install_toolchain(lock, env):
    toolchain = lock["toolchain"]
    destination = Path("/opt/apk-engine-nik")
    if destination.exists():
        raise FileExistsError(str(destination))
    with tempfile.TemporaryDirectory(prefix="apk-engine-nik-") as directory:
        archive = Path(directory) / "nik.tar.gz"
        digest = hashlib.sha256()
        size = 0
        with urllib.request.urlopen(toolchain["url"], timeout=180) as response, archive.open("wb") as output:
            while block := response.read(1024 * 1024):
                output.write(block)
                digest.update(block)
                size += len(block)
        if digest.hexdigest() != toolchain["sha256"] or size != toolchain["bytes"]:
            raise ValueError("Native toolchain download did not match locked SHA-256 and size")
        destination.mkdir(parents=True)
        run(["tar", "-xzf", archive, "--strip-components=1", "-C", destination], "toolchain-extract.log", env)
    env["JAVA_HOME"] = str(destination)
    env["PATH"] = str(destination / "bin") + os.pathsep + env["PATH"]
    return destination


def package(lock, binary, toolchain_root):
    name = lock["artifact"]["name"]
    output = ROOT / "dist" / (name + ".tar.xz")
    with tempfile.TemporaryDirectory(prefix="apk-engine-package-") as directory:
        base = Path(directory) / "opt/apk-engine-toolkit"
        (base / "bin").mkdir(parents=True)
        shutil.copy2(binary, base / "bin/apk-engine")
        (base / "bin/apk-engine").chmod(0o755)
        information = base / "share/build-info"
        information.mkdir(parents=True)
        names = ["native-build.json", "dependency-preparation.json", "integration-jvm.json", "integration-native.json", "elf.json", "native-image.log", "toolchain-version.log", "readelf.log", "file.log", "system-packages.log"]
        for name in names:
            shutil.copy2(EVIDENCE / name, information / name)
        shutil.copy2(ROOT / "dist/build.json", information / "java-build.json")
        shutil.copy2(ROOT / "native-dependencies.json", information / "native-dependencies.json")
        shutil.copytree(ROOT / "ci/generated-config", information / "native-image-config")
        licenses = base / "share/licenses"
        shutil.copytree(ROOT / "ci/licenses", licenses)
        shutil.copy2(ROOT / "ci/vendor-src/axml/LICENSE", licenses / "axml-GPL-3.0.txt")
        shutil.copy2(ROOT / "ci/vendor-src/zipalign-java/MIT.txt", licenses / "zipalign-java-MIT.txt")
        shutil.copy2(ROOT / "ci/vendor-src/zipalign-java/THIRD-PARTY-NOTICES.txt", licenses / "zipalign-java-THIRD-PARTY-NOTICES.txt")
        if (toolchain_root / "legal").is_dir():
            shutil.copytree(toolchain_root / "legal", licenses / "liberica-nik", symlinks=False)
        source = base / "share/source"
        source.mkdir(parents=True)
        for relative in ["src", "tests/verify_toolkit.py", "build.py", "README.md", "LICENSE", "native-dependencies.json", "ci/prepare.py", "ci/check_elf.py", "ci/native_build.py", "ci/vendor-src", "ci/licenses"]:
            original = ROOT / relative
            target = source / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            if original.is_dir():
                shutil.copytree(original, target)
            else:
                shutil.copy2(original, target)
        with tarfile.open(output, "w:xz", preset=6) as archive:
            for path in sorted(Path(directory).rglob("*")):
                info = archive.gettarinfo(str(path), arcname=path.relative_to(directory).as_posix())
                info.uid = 0
                info.gid = 0
                info.uname = ""
                info.gname = ""
                info.mtime = 0
                if path.is_file():
                    with path.open("rb") as contents:
                        archive.addfile(info, contents)
                else:
                    archive.addfile(info)
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_suffix(output.suffix + ".sha256").write_text(digest + "  " + output.name + "\n")
    return {"name": output.name, "bytes": output.stat().st_size, "sha256": digest}


def main():
    EVIDENCE.mkdir(exist_ok=True)
    lock_path = ROOT / "native-dependencies.json"
    lock = json.loads(lock_path.read_text())
    report = {"schema": "apk-engine.native-build/1", "status": "running", "actual_target": lock["artifact"]["actual_target"], "publisher_platform": lock["artifact"]["publisher_platform"], "android_device_validation": "pending", "source_commit": os.environ.get("GITHUB_SHA"), "repository": os.environ.get("GITHUB_REPOSITORY"), "workflow_run_id": os.environ.get("GITHUB_RUN_ID"), "lock_sha256": hashlib.sha256(lock_path.read_bytes()).hexdigest(), "toolchain": lock["toolchain"], "commands": COMMANDS}
    target = EVIDENCE / "native-build.json"
    try:
        if platform.machine() not in {"aarch64", "arm64"}:
            raise RuntimeError("Native build requires an ARM64 host")
        env = dict(os.environ)
        for name in ["JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "JDK_JAVA_OPTIONS", "CLASSPATH"]:
            env.pop(name, None)
        toolchain_root = install_toolchain(lock, env)
        run(["java", "-version"], "toolchain-version.log", env)
        run(["apk", "info", "-v"], "system-packages.log", env)
        library = ROOT / ".ci-library"
        run(["python3", "ci/prepare.py", "--library", library, "--evidence", EVIDENCE / "dependency-preparation.json"], "preparation.log", env)
        run(["python3", "build.py", "--library", library], "java-build.log", env)
        configuration = ROOT / "ci/generated-config"
        configuration.mkdir(parents=True)
        run(["python3", "tests/verify_toolkit.py", "--java-jar", ROOT / "dist/apk-engine.jar", "--java-option=-Xmx512m", "--java-option=-XX:ActiveProcessorCount=2", "--java-option=-agentlib:native-image-agent=config-merge-dir=" + str(configuration), "--portable-fixtures", "--evidence", EVIDENCE / "integration-jvm.json"], "integration-jvm.log", env)
        if not list(configuration.rglob("*.json")):
            raise RuntimeError("Native image agent produced no configuration")
        binary = ROOT / "dist/apk-engine-aarch64"
        run(["native-image", "-J-Xmx4g", "-J-XX:ActiveProcessorCount=2", "--parallelism=2", "--no-fallback", "-Ob", "--static", "--libc=musl", "-march=compatibility", "-H:ConfigurationFileDirectories=" + str(configuration), "-jar", ROOT / "dist/apk-engine.jar", binary], "native-image.log", env)
        run(["python3", "ci/check_elf.py", binary, "--evidence", EVIDENCE / "elf.json"], "elf-check.log", env)
        run(["python3", "tests/verify_toolkit.py", "--executable", binary, "--portable-fixtures", "--evidence", EVIDENCE / "integration-native.json"], "integration-native.log", env)
        run(["readelf", "-h", "-l", "-d", binary], "readelf.log", env)
        run(["file", binary], "file.log", env)
        report.update({"status": "passed", "native_binary": {"name": binary.name, "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "bytes": binary.stat().st_size}, "jvm_trace_validation": "passed", "native_integration_validation": "passed", "static_elf_validation": "passed"})
        target.write_text(json.dumps(report, indent=2) + "\n")
        report["archive"] = package(lock, binary, toolchain_root)
        target.write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report["archive"]))
    except Exception as error:
        report.update({"status": "failed", "error": type(error).__name__ + ": " + str(error)})
        target.write_text(json.dumps(report, indent=2) + "\n")
        raise


if __name__ == "__main__":
    main()
