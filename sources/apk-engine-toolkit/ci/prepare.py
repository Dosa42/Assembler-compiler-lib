import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def checked(path, expected):
    data = path.read_bytes()
    if len(data) != expected["bytes"] or hashlib.sha256(data).hexdigest() != expected["sha256"]:
        raise ValueError("SHA-256 or size mismatch: " + str(path))
    return data


def contained(root, relative):
    path = Path(relative)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError("Invalid relative dependency path")
    return root / path


def compile_module(module, library, work):
    sources = sorted(contained(ROOT, module["source_root"]).rglob("*.java"))
    if not sources:
        raise ValueError("No Java source for " + module["id"])
    classes = work / module["id"]
    classes.mkdir()
    command = ["javac", "-J-Xmx384m", "-J-XX:ActiveProcessorCount=2", "--release", str(module["java_release"]), "-Xlint:-options", "-encoding", "UTF-8", "-d", str(classes)]
    if module["id"] == "axml":
        command.append("-g:none")
    if module["classpath"]:
        command.extend(["-cp", os.pathsep.join(str(contained(library, path)) for path in module["classpath"])])
    result = subprocess.run(command + [str(path) for path in sources], capture_output=True, text=True, timeout=120)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    manifest = "Manifest-Version: 1.0\nMain-Class: " + module["manifest_main_class"] + "\n"
    if module["id"] == "axml":
        manifest += "Class-Path: kxml2-2.3.0.jar\n"
    entries = {"META-INF/MANIFEST.MF": (manifest + "\n").encode()}
    entries.update({path.relative_to(classes).as_posix(): path.read_bytes() for path in classes.rglob("*.class")})
    entries.update({name: contained(ROOT, source).read_bytes() for name, source in module["resources"].items()})
    destination = contained(library, module["output"])
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name, data in sorted(entries.items()):
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, data)
    return {"id": module["id"], "mode": "compiled-pinned-source", "source_files": len(sources), "class_files": sum(name.endswith(".class") for name in entries), "artifact": module["output"], "bytes": destination.stat().st_size, "sha256": hashlib.sha256(destination.read_bytes()).hexdigest(), "compiler_stdout": result.stdout, "compiler_stderr": result.stderr}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--library", type=Path, default=ROOT / ".ci-library")
    parser.add_argument("--offline-source", type=Path)
    parser.add_argument("--reuse-local-builds", action="store_true")
    parser.add_argument("--evidence", type=Path, default=ROOT / "ci/evidence/preparation.json")
    args = parser.parse_args()
    if args.reuse_local_builds and args.offline_source is None:
        raise ValueError("--reuse-local-builds requires --offline-source")
    library = args.library.resolve()
    library.mkdir(parents=True, exist_ok=True)
    lock_path = ROOT / "native-dependencies.json"
    lock = json.loads(lock_path.read_text())
    for record in lock["source_inputs"]:
        checked(contained(ROOT, record["path"]), record)
    prepared = []
    for record in lock["published_jars"]:
        destination = contained(ROOT if record["scope"] == "project" else library, record["destination"])
        if destination.exists():
            checked(destination, record)
            mode = "existing-verified"
        else:
            if args.offline_source is not None:
                if record["scope"] == "project":
                    raise FileNotFoundError("Offline project dependency missing: " + str(destination))
                data = checked(contained(args.offline_source.resolve(), record["local_library_path"]), record)
                mode = "offline-copy"
            else:
                with urllib.request.urlopen(record["url"], timeout=120) as response:
                    data = response.read(record["bytes"] + 1)
                if len(data) != record["bytes"] or hashlib.sha256(data).hexdigest() != record["sha256"]:
                    raise ValueError("Downloaded dependency mismatch: " + record["url"])
                mode = "downloaded-verified"
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
        with zipfile.ZipFile(destination) as archive:
            if archive.testzip() is not None:
                raise ValueError("JAR CRC mismatch: " + str(destination))
        prepared.append({"destination": record["destination"], "mode": mode, "sha256": record["sha256"], "bytes": record["bytes"]})
    built = []
    with tempfile.TemporaryDirectory(prefix=".prepare-", dir=library) as directory:
        work = Path(directory)
        for module in lock["custom_java_modules"]:
            if args.reuse_local_builds:
                record = module["local_reference"]
                source = contained(args.offline_source.resolve(), record["path"])
                data = checked(source, record)
                destination = contained(library, module["output"])
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(data)
                built.append({"id": module["id"], "mode": "verified-local-reference-copy", "artifact": module["output"], "sha256": record["sha256"], "bytes": record["bytes"]})
            else:
                built.append(compile_module(module, library, work))
    report = {"status": "passed", "lock_sha256": hashlib.sha256(lock_path.read_bytes()).hexdigest(), "library": str(library), "source_inputs_verified": len(lock["source_inputs"]), "published_jars": prepared, "custom_modules": built, "custom_source_compiled": not args.reuse_local_builds, "offline": args.offline_source is not None}
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    args.evidence.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"status": "passed", "published_jars": len(prepared), "custom_modules": len(built), "custom_source_compiled": report["custom_source_compiled"], "library": str(library)}))


if __name__ == "__main__":
    main()
