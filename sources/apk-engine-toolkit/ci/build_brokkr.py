import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile

import jsonschema

from check_elf import inspect as inspect_elf


ROOT = Path(__file__).resolve().parents[1]
VENDOR = ROOT / "ci/vendor-src/brokkr"
IDENTITY_SCHEMA = {
    "type": "object", "additionalProperties": False, "required": ["path", "bytes", "sha256"],
    "properties": {"path": {"type": "string", "minLength": 1}, "bytes": {"type": "integer", "minimum": 0},
                   "sha256": {"type": "string", "pattern": "^[a-f0-9]{64}$"}},
}
SOURCE_SCHEMA = {
    "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object", "additionalProperties": False,
    "required": ["schema", "producer", "files"],
    "properties": {"schema": {"const": "apk-engine.brokkr-source/1"}, "producer": {"const": "ci/build_brokkr.py"},
                   "files": {"type": "array", "minItems": 1, "items": IDENTITY_SCHEMA}},
}
ELF_SCHEMA = {
    "type": "object", "additionalProperties": False,
    "required": ["status", "format", "architecture", "pt_interp", "dt_needed", "bytes", "sha256"],
    "properties": {"status": {"const": "passed"}, "format": {"const": "ELF64 little-endian"},
                   "architecture": {"const": "AArch64"}, "pt_interp": {"const": False}, "dt_needed": {"const": []},
                   "bytes": {"type": "integer", "minimum": 1}, "sha256": IDENTITY_SCHEMA["properties"]["sha256"]},
}
BUILD_SCHEMA = {
    "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object", "additionalProperties": False,
    "required": ["schema", "producer", "target", "source_manifest", "compiler", "libusb_version", "libusb_static_flags",
                 "binaries", "static_library", "elf", "broker_fixture_validation", "broker_metadata_validation", "device_operations"],
    "properties": {
        "schema": {"const": "apk-engine.brokkr-build/1"}, "producer": {"const": "ci/build_brokkr.py"},
        "target": {"const": "aarch64-linux-musl-static"}, "source_manifest": IDENTITY_SCHEMA,
        "compiler": {"type": "string", "minLength": 1}, "libusb_version": {"type": "string", "minLength": 1},
        "libusb_static_flags": {"type": "string", "minLength": 1}, "static_library": IDENTITY_SCHEMA,
        "broker_fixture_validation": {"const": "passed"}, "broker_metadata_validation": {"const": "passed"},
        "device_operations": {"const": False},
        "binaries": {"type": "object", "additionalProperties": False, "required": ["brokkr-engine", "brokkr-usb-broker"],
                     "properties": {name: IDENTITY_SCHEMA for name in ["brokkr-engine", "brokkr-usb-broker"]}},
        "elf": {"type": "object", "additionalProperties": False, "required": ["brokkr-engine", "brokkr-usb-broker"],
                "properties": {name: ELF_SCHEMA for name in ["brokkr-engine", "brokkr-usb-broker"]}},
    },
}
SOURCE_FILES = [
    "CMakeLists.txt", "build.py", "verify.py", "README.md", "fixtures/generate.py", "examples/native_client.c",
    "upstream/provenance.json", "usb/Makefile", "usb/LICENSE", "usb/README.md", "usb/metadata.schema.json",
    "usb/brokkr_usb_broker.c", "usb/brokkr_usb_client.c", "usb/brokkr_usb_client.h", "usb/tests/test_usb_broker.c",
    "usb/tests/check_metadata.py",
]
SOURCE_TREES = ["include", "src", "schemas", "licenses", "upstream/original", "upstream/dependencies"]
SOURCE_SUFFIXES = {".c", ".cpp", ".h", ".hpp", ".asm", ".S", ".json", ".md", ".txt", ".TXT", ".sh", ".patch", ".bz2", ".cmake"}
EXCLUDED_PARTS = {".git", ".build", "build", "bin", "lib", "evidence", "__pycache__", ".cache", "data"}


def identity(path, base):
    return {"path": path.relative_to(base).as_posix(), "bytes": path.stat().st_size,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def write_record(path, record, schema):
    jsonschema.Draft202012Validator.check_schema(schema)
    jsonschema.Draft202012Validator(schema).validate(record)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(record, indent=2) + "\n")
    path.with_suffix(".schema.json").write_text(json.dumps(schema, indent=2) + "\n")


def selected_sources(source):
    paths = {source / relative for relative in SOURCE_FILES}
    for relative in SOURCE_TREES:
        tree = source / relative
        if not tree.is_dir():
            raise FileNotFoundError(tree)
        for path in tree.rglob("*"):
            parts = path.relative_to(source).parts
            if any(part in EXCLUDED_PARTS for part in parts):
                continue
            if path.is_file() and (path.suffix in SOURCE_SUFFIXES or path.name in {"LICENSE", "CMakeLists.txt"}):
                paths.add(path)
    for path in sorted(paths):
        if path.is_symlink() or not path.is_file() or not path.resolve().is_relative_to(source):
            raise ValueError("Expected an ordinary source file inside the engine family: " + str(path))
    return sorted(paths)


def stage(source, destination):
    """Stage selected sources after the engine owner freezes them; never copy installed binaries or captures."""
    source = source.resolve()
    destination = destination.absolute()
    if destination.exists():
        raise FileExistsError(destination)
    paths = selected_sources(source)
    records = [identity(path, source) for path in paths]
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".brokkr-stage-", dir=destination.parent) as directory:
        staged = Path(directory) / "brokkr"
        staged.mkdir()
        for path in paths:
            target = staged / path.relative_to(source)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
        manifest = {"schema": "apk-engine.brokkr-source/1", "producer": "ci/build_brokkr.py", "files": records}
        write_record(staged / "ci-source.json", manifest, SOURCE_SCHEMA)
        verify_sources(staged)
        staged.rename(destination)
    return manifest


def verify_sources(source):
    manifest = json.loads((source / "ci-source.json").read_text())
    jsonschema.Draft202012Validator(SOURCE_SCHEMA).validate(manifest)
    seen = set()
    for record in manifest["files"]:
        relative = Path(record["path"])
        if relative.is_absolute() or ".." in relative.parts or record["path"] in seen:
            raise ValueError("Invalid or duplicate staged source path")
        path = source / relative
        if path.is_symlink() or not path.is_file() or not path.resolve().is_relative_to(source.resolve()):
            raise ValueError("Missing or escaping staged source: " + str(relative))
        if identity(path, source) != record:
            raise ValueError("Staged source identity changed: " + str(relative))
        seen.add(record["path"])
    paths = list(source.rglob("*"))
    if any(path.is_symlink() for path in paths):
        raise ValueError("Staged source tree contains a symbolic link")
    actual = {path.relative_to(source).as_posix() for path in paths if path.is_file()}
    if actual != seen | {"ci-source.json", "ci-source.schema.json"}:
        raise ValueError("Staged source tree has unrecorded files")
    if json.loads((source / "ci-source.schema.json").read_text()) != SOURCE_SCHEMA:
        raise ValueError("Staged source schema differs from its producer")
    return manifest


def build(library, evidence, env, run):
    verify_sources(VENDOR)
    machine = subprocess.check_output(["gcc", "-dumpmachine"], text=True, env=env).strip()
    if platform.machine() not in {"aarch64", "arm64"} or not machine.startswith("aarch64-") or not machine.endswith("-linux-musl"):
        raise RuntimeError("Brokkr CI build requires an AArch64 musl GCC toolchain")
    work = ROOT / ".ci-brokkr"
    shutil.copytree(VENDOR, work)
    (work / "evidence").mkdir()
    run([sys.executable, work / "build.py", "--static", "--compiler", "g++", "--c-compiler", "gcc"], "brokkr-build.log", env)
    run(["make", "-C", work / "usb", "CC=gcc", "CFLAGS=-O2 -g -static", "check"], "brokkr-usb-build.log", env)
    run([sys.executable, work / "usb/tests/check_metadata.py"], "brokkr-usb-metadata.log", env)
    outputs = {"brokkr-engine": work / "bin/brokkr-engine",
               "brokkr-usb-broker": work / "usb/bin/brokkr-usb-broker"}
    binaries, elves = {}, {}
    (ROOT / "dist").mkdir(exist_ok=True)
    for name, original in outputs.items():
        elves[name] = inspect_elf(original)
        target = ROOT / "dist" / name
        shutil.copy2(original, target)
        target.chmod(0o755)
        binaries[name] = identity(target, ROOT)
        run(["readelf", "-h", "-l", "-d", target], name + "-readelf.log", env)
    result_schema = VENDOR / "schemas/result.schema.json"
    jsonschema.Draft202012Validator.check_schema(json.loads(result_schema.read_text()))
    schema_target = library / "brokkr/schemas/result.schema.json"
    schema_target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(result_schema, schema_target)
    report = {
        "schema": "apk-engine.brokkr-build/1", "producer": "ci/build_brokkr.py", "target": "aarch64-linux-musl-static",
        "source_manifest": identity(VENDOR / "ci-source.json", ROOT),
        "compiler": subprocess.check_output(["g++", "--version"], text=True, env=env).splitlines()[0],
        "libusb_version": subprocess.check_output(["pkg-config", "--modversion", "libusb-1.0"], text=True, env=env).strip(),
        "libusb_static_flags": subprocess.check_output(["pkg-config", "--static", "--libs", "libusb-1.0"], text=True, env=env).strip(),
        "binaries": binaries, "static_library": identity(work / "lib/libbrokkr-engine.a", ROOT), "elf": elves,
        "broker_fixture_validation": "passed", "broker_metadata_validation": "passed", "device_operations": False,
    }
    write_record(evidence / "brokkr-build.json", report, BUILD_SCHEMA)
    return report


def main():
    parser = argparse.ArgumentParser(description="Stage frozen Brokkr source for the native CI build.")
    subparsers = parser.add_subparsers(dest="command", required=True)
    staging = subparsers.add_parser("stage")
    staging.add_argument("--source", type=Path, required=True)
    staging.add_argument("--destination", type=Path, default=VENDOR)
    verify = subparsers.add_parser("verify-source")
    verify.add_argument("--source", type=Path, default=VENDOR)
    args = parser.parse_args()
    if args.command == "stage":
        record = stage(args.source, args.destination)
        print(f"staged {len(record['files'])} source files at {args.destination}")
    else:
        record = verify_sources(args.source)
        print(f"verified {len(record['files'])} staged source files")


if __name__ == "__main__":
    main()
