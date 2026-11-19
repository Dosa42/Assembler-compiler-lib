import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shlex
import subprocess
import struct
import sys
import tarfile
from datetime import datetime, timezone

import jsonschema

sys.dont_write_bytecode = True
from fixtures.generate import generate


ROOT = Path(__file__).resolve().parent


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--binary", type=Path, default=ROOT / "bin/brokkr-engine")
    parser.add_argument("--library", type=Path, default=ROOT / "lib/libbrokkr-engine.so")
    args = parser.parse_args()
    fixtures = ROOT / "fixtures/data"
    expected = generate(fixtures)
    result_schema = json.loads((ROOT / "schemas/result.schema.json").read_text())
    evidence_schema = json.loads((ROOT / "schemas/offline-verification.schema.json").read_text())
    jsonschema.Draft202012Validator.check_schema(result_schema)
    jsonschema.Draft202012Validator.check_schema(evidence_schema)
    validator = jsonschema.Draft202012Validator(result_schema)
    checks = []
    native_env = {key: value for key, value in os.environ.items() if key not in ("LD_LIBRARY_PATH", "LD_PRELOAD")}
    source_paths = [Path(__file__), ROOT / "fixtures/generate.py", ROOT / "examples/native_client.c",
                    ROOT / "schemas/result.schema.json", ROOT / "schemas/offline-verification.schema.json",
                    ROOT / "include/brokkr_engine.h", args.binary, args.library]
    if args.candidate:
        source_paths.append(args.candidate)
    source_paths += sorted(fixtures.iterdir())
    before_hashes = {path: sha256(path) for path in source_paths if path.is_file()}

    def check(identifier, interface, action):
        try:
            detail = action()
            checks.append({"id": identifier, "interface": interface, "ok": True, "detail": detail or "passed"})
            return True
        except Exception as error:
            checks.append({"id": identifier, "interface": interface, "ok": False, "detail": str(error)})
            return False

    def run(executable, argv, success=True, error_code=None):
        completed = subprocess.run([str(executable), *map(str, argv)], capture_output=True, text=True, timeout=30, env=native_env)
        expected_status = 0 if success else 2 if error_code == "usage" else 1
        require(completed.returncode == expected_status,
                f"unexpected exit {completed.returncode}: {completed.stderr.strip()} {completed.stdout[:800]}")
        require(bool(completed.stdout.strip()), "missing JSON response: " + completed.stderr.strip())
        payload = json.loads(completed.stdout)
        validator.validate(payload)
        require(payload["ok"] is success, f"unexpected ok value: {payload}")
        if argv:
            require(payload["operation"] == argv[0], "operation does not match request")
        if error_code:
            require(payload["error"]["code"] == error_code, f"unexpected error code: {payload['error']['code']}")
        return payload

    def fixture_oracle():
        archive = (fixtures / "fixture.tar").read_bytes()
        require(len(archive) == 3072, "unexpected fixed archive length")
        require(hashlib.md5(archive).hexdigest() == expected["md5"], "independent MD5 mismatch")
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as parsed:
            entries = [(item.name, item.size, item.offset_data) for item in parsed]
        require(entries == [("images/boot.img", expected["boot_bytes"], 512),
                            ("vbmeta.img", expected["vbmeta_bytes"], 1536)], "USTAR oracle mismatch")
        pit = (fixtures / "sample.pit").read_bytes()
        require(len(pit) == 292 and struct.unpack_from("<II", pit, 0) == (0x12349876, 2), "PIT wire header mismatch")
        require(struct.unpack_from("<I", pit, 36)[0] == 7 and struct.unpack_from("<I", pit, 56)[0] == 2048,
                "PIT first partition fixed offsets mismatch")
        require(pit[64:68] == b"BOOT" and pit[96:104] == b"boot.img", "PIT fixed name offsets mismatch")
        require(struct.unpack_from("<I", pit, 168)[0] == 9 and struct.unpack_from("<I", pit, 188)[0] == 4096,
                "PIT second partition fixed offsets mismatch")
        decoded = subprocess.run(["lz4", "-d", "-c", str(fixtures / "compressed.img.lz4")],
                                 capture_output=True, timeout=30, env=native_env)
        require(decoded.returncode == 0 and decoded.stdout == (fixtures / "boot.img").read_bytes() * 8,
                "system LZ4 decoder rejected the fixed compressed fixture")
        for name in ("corrupt-compressed.img.lz4", "bad-header.img.lz4", "missing-footer.img.lz4"):
            rejected = subprocess.run(["lz4", "-d", "-c", str(fixtures / name)],
                                      capture_output=True, timeout=30, env=native_env)
            require(rejected.returncode != 0, "system LZ4 decoder accepted malformed fixture: " + name)
        return "Python USTAR reader, hashlib MD5, explicit PIT offsets and system LZ4 decoder agree"

    check("independent-fixture-oracle", "fixture", fixture_oracle)

    def policy_check():
        rules = Path.home() / ".codex/RULES"
        jsonschema.validate(json.loads((rules / "comment-documentation.json").read_text()),
                            json.loads((rules / "comment-documentation.schema.json").read_text()))
        return "canonical comment policy validates against its consumer schema"

    check("canonical-policy-schema", "policy", policy_check)

    def schema_rejects_invalid():
        require(not validator.is_valid({"schema": "brokkr.result/1", "ok": True,
                                        "operation": "parse-pit", "result": {}}), "schema accepts missing PIT fields")
        require(not validator.is_valid({"schema": "brokkr.result/1", "ok": False,
                                        "operation": "parse-pit", "error": {"code": "", "message": ""}}),
                "schema accepts empty errors")
        return "consumer schema rejects incomplete success and empty error envelopes"

    check("schema-rejects-invalid-envelopes", "schema", schema_rejects_invalid)

    def expect_pit(result):
        require((result["com_tar2"], result["cpu_bl_id"], result["lu_count"], result["is_ab"],
                 result["common_block_bytes"]) == ("COM_TAR2", "TESTCPU", 1, False, 512), "PIT header mismatch")
        fields = ("id", "name", "file_name", "begin_block", "block_count", "capacity_bytes", "wire_block_size")
        actual = [tuple(item[key] for key in fields) for item in result["partitions"]]
        require(actual == [(7, "BOOT", "boot.img", 2048, 2048, 1048576, 512),
                           (9, "VBMETA", "vbmeta.img", 4096, 8, 4096, 512)], f"PIT partition mismatch: {actual}")
        require(all((p["bin_type"], p["dev_type"], p["attribute"], p["block_bytes"]) == (0, 2, 5, 512)
                    for p in result["partitions"]), "PIT type, attribute or block-unit mismatch")

    def expect_ab(result):
        require(result["is_ab"] is True and result["common_block_bytes"] is None, "mixed-device A/B mismatch")
        require([item["capacity_bytes"] for item in result["partitions"]] == [16384, 4096], "device block-size mismatch")

    def expect_tar(result):
        require(result["format"] == "tar" and result["tar_payload_bytes"] == 3072 and result["file_bytes"] == 3072,
                "archive format, file length or end offset mismatch")
        require([(item["name"], item["size_bytes"], item["data_offset"]) for item in result["entries"]] ==
                [("images/boot.img", expected["boot_bytes"], 512), ("vbmeta.img", expected["vbmeta_bytes"], 1536)],
                "archive entries mismatch")
        require([(item["basename"], item["size_bytes"], item["lz4"]) for item in result["images"]] ==
                [("boot.img", expected["boot_bytes"], False), ("vbmeta.img", expected["vbmeta_bytes"], False)],
                "archive images mismatch")

    def expect_raw(result):
        require(result["format"] == "raw" and result["tar_payload_bytes"] is None and not result["entries"], "raw classification mismatch")
        require(len(result["images"]) == 1 and result["images"][0]["size_bytes"] == expected["boot_bytes"], "raw image size mismatch")

    def expect_lz4(result):
        require(result["format"] == "raw" and len(result["images"]) == 1, "LZ4 raw classification mismatch")
        image = result["images"][0]
        require((image["basename"], image["source_basename"], image["size_bytes"], image["disk_bytes"],
                 image["lz4"], image["lz4_block_bytes"]) ==
                ("compressed.img", "compressed.img.lz4", 184, 61, True, 65536), "LZ4 expansion metadata mismatch")

    def expect_verified(result):
        require((result["md5_status"], result["md5_verified"], result["bytes_hashed"], result["images_checked"]) ==
                ("verified", True, 3072, 2), "verified MD5 accounting mismatch")

    def expect_absent(result):
        require((result["md5_status"], result["md5_verified"], result["bytes_hashed"]) == ("absent", False, 0), "absent digest mislabeled")

    def expect_mapping(result):
        require([(item["partition_id"], item["partition_name"], item["size_bytes"], item["capacity_bytes"])
                 for item in result["items"]] == [(7, "BOOT", expected["boot_bytes"], 1048576),
                                                 (9, "VBMETA", expected["vbmeta_bytes"], 4096)], "mapping mismatch")
        require(not result["unmapped"], "unexpected unmapped image")

    cases = [
        ("pit-known-offsets", ["parse-pit", fixtures / "sample.pit"], True, expect_pit, None),
        ("pit-mixed-device-ab", ["parse-pit", fixtures / "mixed-ab.pit"], True, expect_ab, None),
        ("ustar-known-offsets", ["inspect-firmware", fixtures / "fixture.tar"], True, expect_tar, None),
        ("raw-image", ["inspect-firmware", fixtures / "boot.img"], True, expect_raw, None),
        ("lz4-expanded-image", ["inspect-firmware", fixtures / "compressed.img.lz4"], True, expect_lz4, None),
        ("lz4-decoded-stream", ["verify-firmware", fixtures / "compressed.img.lz4"], True, expect_absent, None),
        ("md5-verified", ["verify-firmware", fixtures / "fixture.tar.md5"], True, expect_verified, None),
        ("md5-absent-explicit", ["verify-firmware", fixtures / "fixture.tar"], True, expect_absent, None),
        ("pit-image-mapping", ["map-firmware-to-pit", fixtures / "sample.pit", fixtures / "fixture.tar"], True, expect_mapping, None),
    ]
    for name in ("bad-magic", "negative-count", "inflated-count", "short-header", "short-entry"):
        cases.append((name, ["parse-pit", fixtures / (name + ".pit")], False, None, "invalid_pit"))
    for name, code in (("bad-checksum.tar", "invalid_firmware"), ("truncated.tar", "invalid_firmware"),
                       ("corrupt-payload.tar.md5", "checksum_mismatch"), ("missing-trailer.tar.md5", "missing_md5"),
                       ("corrupt-compressed.img.lz4", "invalid_lz4"), ("bad-header.img.lz4", "invalid_lz4"),
                       ("missing-footer.img.lz4", "invalid_lz4")):
        cases.append((name, ["verify-firmware", fixtures / name], False, None, code))
    cases += [
        ("unmapped-rejected", ["map-firmware-to-pit", fixtures / "sample.pit", fixtures / "unknown.img"], False, None, "mapping_failed"),
        ("missing-file", ["inspect-firmware", fixtures / "does-not-exist.tar"], False, None, "input_error"),
        ("usage-missing-argument", ["parse-pit"], False, None, "usage"),
        ("usage-unknown-command", ["not-an-operation"], False, None, "usage"),
        ("flash-upstream-lz4-flow", ["flash", "--fd", "999999", "--ap", fixtures / "corrupt-compressed.img.lz4"], False, None, "usb_fd"),
    ]
    cli_results = {}
    for identifier, argv, success, expectation, error_code in cases:
        def action(argv=argv, success=success, expectation=expectation, identifier=identifier, error_code=error_code):
            payload = run(args.binary, argv, success, error_code)
            if expectation:
                expectation(payload["result"])
            cli_results[identifier] = payload
            return "schema and fixed oracle passed" if expectation else "structured error and nonzero exit"
        check(identifier, "cli", action)

    client = ROOT / "examples/.build/native-client"
    client.parent.mkdir(exist_ok=True)
    compile_argv = [*shlex.split(os.environ.get("CC", "clang")), "-std=c11", "-Wall", "-Wextra", "-Werror",
                    "-I", str(ROOT / "include"), str(ROOT / "examples/native_client.c"), str(args.library),
                    "-Wl,-rpath," + str(args.library.resolve().parent), "-o", str(client)]

    def compile_client():
        compiled = subprocess.run(compile_argv, capture_output=True, text=True, timeout=60, env=native_env)
        require(compiled.returncode == 0, compiled.stderr.strip())
        patch_argv = ["patchelf", "--set-rpath", str(args.library.resolve().parent), str(client)]
        patched = subprocess.run(patch_argv, capture_output=True, text=True, timeout=30, env=native_env)
        require(patched.returncode == 0, patched.stderr.strip())
        return shlex.join(compile_argv) + "; runtime path: " + shlex.join(patch_argv)

    if check("external-c-client-compiled", "c-api", compile_client):
        for identifier, argv, success, expectation, error_code in cases:
            def action(argv=argv, success=success, identifier=identifier, error_code=error_code):
                payload = run(client, argv, success, error_code)
                require(identifier in cli_results, "CLI case failed; cannot establish parity")
                require(payload == cli_results[identifier], "C API and CLI responses differ")
                return "external C caller matched CLI JSON and released its allocated result"
            check(identifier, "c-api", action)

    candidate = {"requested": args.candidate is not None, "path": str(args.candidate) if args.candidate else None,
                 "tested": False, "detail": "not requested"}
    if args.candidate:
        def candidate_check():
            operation = "parse-pit" if args.candidate.suffix.lower() == ".pit" else "inspect-firmware"
            run(args.binary, [operation, args.candidate])
            if operation == "inspect-firmware":
                verification = run(args.binary, ["verify-firmware", args.candidate])["result"]
                candidate["detail"] = ("offline structure validated; md5_status=" + verification["md5_status"] +
                                       "; images_checked=" + str(verification["images_checked"]))
            else:
                candidate["detail"] = "offline PIT structure validated"
            candidate["tested"] = True
            return candidate["detail"]
        if not check("optional-private-candidate", "cli", candidate_check):
            candidate["detail"] = checks[-1]["detail"]

    def stable_artifacts():
        for path in source_paths:
            require(path in before_hashes and path.is_file() and sha256(path) == before_hashes[path],
                    "artifact changed or was missing during verification: " + str(path))
        return "source, schemas, C header, CLI, shared library, fixtures and selected candidate hashes unchanged during tests"

    check("artifact-bytes-stable", "fixture", stable_artifacts)
    artifact_paths = [*source_paths, client]
    evidence = {
        "schema": "brokkr.offline-verification/1", "producer": "brokkr/verify.py",
        "generated_at": datetime.now(timezone.utc).isoformat(), "ok": all(item["ok"] for item in checks),
        "checks": checks,
        "artifacts": [{"path": str(path.resolve()), "sha256": sha256(path)} for path in artifact_paths if path.is_file()],
        "candidate": candidate,
        "semantic_review": "Fixed expected values originate from independent synthetic bytes. Success requires schema validity, exact parsed values, malformed-input rejection and external C parity. The flash error case uses an invalid inherited descriptor and exits before libusb acquisition; no device communication occurs.",
        "limitations": ["Synthetic offline tests do not validate device transport, flashing, bootability or firmware compatibility.",
                        "The default run does not inspect a private firmware candidate."] if not args.candidate else
                       ["Offline checks do not validate device transport, flashing, bootability or firmware compatibility."],
    }
    jsonschema.Draft202012Validator(evidence_schema, format_checker=jsonschema.FormatChecker()).validate(evidence)
    output = ROOT / "evidence/offline-verification.json"
    output.parent.mkdir(exist_ok=True)
    output.write_text(json.dumps(evidence, indent=2) + "\n")
    failed = [item for item in checks if not item["ok"]]
    print(json.dumps({"ok": evidence["ok"], "passed": len(checks) - len(failed), "failed_count": len(failed),
                      "failed": failed[:5], "evidence": str(output)}, indent=2))
    return 0 if evidence["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
