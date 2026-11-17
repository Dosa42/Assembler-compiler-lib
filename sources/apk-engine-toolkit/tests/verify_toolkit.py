import argparse
import hashlib
import importlib.util
import json
import os
import secrets
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ET
import zipfile
from datetime import datetime, timezone
from pathlib import Path


PROJECT = Path(__file__).resolve().parents[1]
TOOLS = {"inspect_apk", "list_entries", "extract_entries", "create_archive", "align_apk", "check_alignment", "list_dex_classes", "disassemble_dex", "assemble_dex", "decode_xml", "encode_xml", "inspect_resources", "verify_apk", "sign_apk"}
PAYLOAD = b"apk-engine-integration-payload-unique-v1\n"


def digest(path):
    result = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for data in iter(lambda: stream.read(1048576), b""):
            result.update(data)
    return result.hexdigest()


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def zip_contents(path):
    with zipfile.ZipFile(path) as archive:
        require(archive.testzip() is None, "ZIP CRC failed")
        return {entry.filename: archive.read(entry) for entry in archive.infolist() if not entry.is_dir()}


def offsets(path):
    data = Path(path).read_bytes()
    items = []
    with zipfile.ZipFile(path) as archive:
        for entry in archive.infolist():
            if not entry.is_dir() and entry.compress_type == zipfile.ZIP_STORED:
                name, extra = struct.unpack_from("<HH", data, entry.header_offset + 26)
                offset = entry.header_offset + 30 + name + extra
                alignment = 16384 if entry.filename.endswith(".so") else 4
                items.append({"name": entry.filename, "offset": offset, "alignment": alignment, "aligned": offset % alignment == 0})
    return items


def binary_chunk(kind, header_size, body):
    return struct.pack("<HHI", kind, header_size, len(body) + 8) + body


def string_pool(values):
    data = bytearray()
    offsets = []
    for value in values:
        encoded = value.encode()
        offsets.append(len(data))
        require(len(encoded) < 128, "Fixture string exceeds short UTF-8 length encoding")
        data.extend(bytes([len(value), len(encoded)]) + encoded + b"\0")
    data.extend(b"\0" * (-len(data) % 4))
    return binary_chunk(1, 28, struct.pack("<IIIII", len(values), 0, 256, 28 + 4 * len(values), 0) + b"".join(struct.pack("<I", offset) for offset in offsets) + data)


def binary_manifest():
    pool = string_pool(["minSdkVersion", "targetSdkVersion", "android", "http://schemas.android.com/apk/res/android", "manifest", "package", "engine.library.integration", "uses-sdk", "application"])
    resources = binary_chunk(0x0180, 8, struct.pack("<II", 0x0101020C, 0x01010270))
    node = struct.pack("<II", 1, 0xFFFFFFFF)

    def attribute(namespace, name, raw, kind, value):
        return struct.pack("<IIIHBBI", namespace, name, raw, 8, 0, kind, value)

    def start(name, attributes):
        return binary_chunk(0x0102, 16, node + struct.pack("<IIHHHHHH", 0xFFFFFFFF, name, 20, 20, len(attributes), 0, 0, 0) + b"".join(attributes))

    def end(name):
        return binary_chunk(0x0103, 16, node + struct.pack("<II", 0xFFFFFFFF, name))

    body = pool + resources + binary_chunk(0x0100, 16, node + struct.pack("<II", 2, 3))
    body += start(4, [attribute(0xFFFFFFFF, 5, 6, 3, 6)])
    body += start(7, [attribute(3, 0, 0xFFFFFFFF, 16, 24), attribute(3, 1, 0xFFFFFFFF, 16, 35)]) + end(7)
    body += start(8, []) + end(8) + end(4) + binary_chunk(0x0101, 16, node + struct.pack("<II", 2, 3))
    return binary_chunk(3, 8, body)


def resource_table():
    global_pool = string_pool(["fixture-first", "fixture-second", "fixture-third"])
    type_pool = string_pool(["string"])
    key_pool = string_pool(["first", "second", "third"])
    type_spec = binary_chunk(0x0202, 16, struct.pack("<BBHI", 1, 0, 0, 3) + b"\0" * 12)
    entries = b"".join(struct.pack("<HHIHBBI", 8, 0, index, 8, 0, 3, index) for index in range(3))
    type_body = struct.pack("<BBHII", 1, 0, 0, 3, 96) + struct.pack("<I", 64) + b"\0" * 60 + struct.pack("<III", 0, 16, 32) + entries
    type_chunk = binary_chunk(0x0201, 84, type_body)
    package_name = "engine.library.integration".encode("utf-16le").ljust(256, b"\0")
    package_header = struct.pack("<I", 0x7F) + package_name + struct.pack("<IIIII", 288, 1, 288 + len(type_pool), 3, 0)
    package = binary_chunk(0x0200, 288, package_header + type_pool + key_pool + type_spec + type_chunk)
    return binary_chunk(2, 12, struct.pack("<I", 1) + global_pool + package)


class Suite:
    def __init__(self, args, work):
        self.args = args
        self.work = work
        self.target = Path(args.executable or args.java_jar).resolve()
        self.prefix = [str(self.target)] if args.executable else [shutil.which("java") or "java", "-Xmx256m", "-XX:ActiveProcessorCount=2"] + args.java_option + ["-jar", str(self.target)]
        self.environment = os.environ.copy()
        self.environment["APK_ENGINE_KEYSTORE_PASSWORD"] = secrets.token_urlsafe(24)
        self.environment.pop("APK_ENGINE_TEST_UNSET_PASSWORD", None)
        self.calls = []
        self.cases = []
        self.cli_tools = set()
        self.mcp_tools = set()
        self.catalog = None
        self.protected = {}

    def process(self, arguments, stdin=None, expected=0, prefix=None):
        command = (self.prefix if prefix is None else prefix) + arguments
        environment = self.environment
        jvm_hidden = bool(self.args.executable and prefix is None)
        if jvm_hidden:
            environment = self.environment.copy()
            environment["PATH"] = "/nonexistent"
            for key in ["JAVA_HOME", "JDK_HOME", "GRAALVM_HOME", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "JDK_JAVA_OPTIONS", "CLASSPATH"]:
                environment.pop(key, None)
        start = time.monotonic()
        run = subprocess.run(command, input=stdin, text=True, capture_output=True, env=environment, cwd=self.work, timeout=120)
        record = {"command": command, "exit_code": run.returncode, "elapsed_seconds": round(time.monotonic() - start, 3), "stdout": run.stdout, "stderr": run.stderr[:8192], "stderr_bytes": len(run.stderr.encode()), "stderr_truncated": len(run.stderr) > 8192, "jvm_hidden_from_child": jvm_hidden}
        self.calls.append(record)
        require(run.returncode == expected, "Unexpected exit " + str(run.returncode) + ": " + json.dumps({"command": command, "stdout": run.stdout[:500], "stderr": run.stderr[:1500]}))
        return record

    def cli(self, name, arguments, expected=0, mode="json"):
        payload = json.dumps(arguments)
        stdin = None
        if mode == "file":
            path = self.work / "arguments.json"
            path.write_text(payload)
            argument = str(path)
        elif mode == "stdin":
            argument = "-"
            stdin = payload
        else:
            argument = payload
        record = self.process([name.replace("_", "-"), argument], stdin, expected)
        response = json.loads(record["stdout"])
        if expected:
            require("error" in response, "CLI failure lacks JSON error")
            return response
        require(response["tool"] == name and response["schema"] == "apk-engine.result/1", "Wrong CLI envelope")
        self.cli_tools.add(name)
        return response["result"]

    def case(self, name, action):
        start = time.monotonic()
        try:
            action()
            result = {"name": name, "passed": True}
        except Exception as error:
            result = {"name": name, "passed": False, "error": type(error).__name__ + ": " + str(error)}
        result["elapsed_seconds"] = round(time.monotonic() - start, 3)
        self.cases.append(result)
        print(json.dumps(result), flush=True)

    def prepare(self):
        if self.args.executable:
            with self.target.open("rb") as stream:
                require(stream.read(4) == b"\x7fELF", "--executable requires a native ELF file")
        self.protected[str(self.target)] = digest(self.target)
        self.smali = self.work / "smali"
        self.smali.mkdir()
        (self.smali / "Counter.smali").write_text(".class public Lfixture/Counter;\n.super Ljava/lang/Object;\n.field public static final VALUE:I = 0x2a\n.method public static answer()I\n    .registers 1\n    const/16 v0, 0x2a\n    return v0\n.end method\n")
        self.xml = self.work / "manifest.xml"
        self.xml.write_text('<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="engine.library.integration" android:versionCode="1" android:versionName="1.0"><uses-sdk android:minSdkVersion="24" android:targetSdkVersion="35"/><application android:label="Fixture" android:debuggable="false"/></manifest>')
        self.manifest = self.work / "fixture.axml"
        self.manifest.write_bytes(binary_manifest())
        self.resources = self.work / "resources.arsc"
        self.resources.write_bytes(resource_table())
        self.dex = self.work / "fixture.dex"
        self.cli("assemble_dex", {"source_dir": str(self.smali), "output": str(self.dex)})
        if self.args.reference_apk:
            self.reference = Path(self.args.reference_apk).resolve()
            self.protected[str(self.reference)] = digest(self.reference)
        self.tree = self.work / "tree"
        (self.tree / "assets").mkdir(parents=True)
        (self.tree / "lib/arm64-v8a").mkdir(parents=True)
        (self.tree / "AndroidManifest.xml").write_bytes(self.manifest.read_bytes())
        (self.tree / "resources.arsc").write_bytes(self.resources.read_bytes())
        (self.tree / "classes.dex").write_bytes(self.dex.read_bytes())
        (self.tree / "assets/payload.txt").write_bytes(PAYLOAD)
        (self.tree / "lib/arm64-v8a/libfixture.so").write_bytes(b"ELF-alignment-test-fixture\0" * 7)
        self.apk = self.work / "fixture.apk"
        with zipfile.ZipFile(self.apk, "w", compression=zipfile.ZIP_STORED) as archive:
            for path in sorted(self.tree.rglob("*")):
                if path.is_file():
                    archive.writestr(path.relative_to(self.tree).as_posix(), path.read_bytes())
        self.protected[str(self.apk)] = digest(self.apk)
        self.store = self.work / "fixture.p12"
        keytool = shutil.which("keytool")
        require(keytool is not None, "keytool is required for ephemeral signing fixture")
        self.process(["-J-Xmx256m", "-J-XX:ActiveProcessorCount=2", "-genkeypair", "-alias", "fixture", "-keyalg", "RSA", "-keysize", "2048", "-sigalg", "SHA256withRSA", "-validity", "2", "-dname", "CN=apk-engine-integration", "-storetype", "PKCS12", "-keystore", str(self.store), "-storepass:env", "APK_ENGINE_KEYSTORE_PASSWORD", "-keypass:env", "APK_ENGINE_KEYSTORE_PASSWORD", "-noprompt"], prefix=[keytool])
        certificate = self.work / "fixture.der"
        self.process(["-exportcert", "-alias", "fixture", "-keystore", str(self.store), "-storepass:env", "APK_ENGINE_KEYSTORE_PASSWORD", "-file", str(certificate)], prefix=[keytool])
        self.certificate_sha256 = digest(certificate)
        self.protected[str(self.store)] = digest(self.store)

    def schemas(self):
        self.catalog = json.loads(self.process(["tools"])["stdout"])
        require({tool["name"] for tool in self.catalog["tools"]} == TOOLS, "Catalog coverage differs from expected tools")
        for tool in self.catalog["tools"]:
            schema = json.loads(self.process([tool["name"], "--schema"])["stdout"])
            require(schema == tool["inputSchema"] and schema["additionalProperties"] is False, "CLI schema differs from catalog")
            require(set(schema["required"]) <= set(schema["properties"]), "Invalid required property")
        require(json.loads(self.process(["--llms"])["stdout"]) == self.catalog, "--llms catalog mismatch")
        require(self.process(["--version"])["stdout"].strip() == self.catalog["version"], "Version mismatch")
        require("Usage:" in self.process(["--help"])["stdout"], "Help missing")

    def archives(self):
        inspect = self.cli("inspect_apk", {"path": str(self.apk)}, mode="file")
        require(inspect["manifest"] and inspect["resources_arsc"] and inspect["dex"] == ["classes.dex"], "APK inspection mismatch")
        require(inspect["native_libraries"] == ["lib/arm64-v8a/libfixture.so"], "Native entry inventory mismatch")
        page = self.cli("list_entries", {"path": str(self.apk), "prefix": "assets/", "limit": 1}, mode="stdin")
        require(page["total"] == 1 and page["entries"][0]["name"] == "assets/payload.txt", "ZIP prefix page mismatch")
        beyond = self.cli("list_entries", {"path": str(self.apk), "offset": 999})
        require(beyond["entries"] == [] and beyond["next_offset"] is None, "Out-of-range ZIP page mismatch")
        selected = self.work / "selected"
        self.cli("extract_entries", {"path": str(self.apk), "output_dir": str(selected), "entries": ["assets/payload.txt"]})
        require((selected / "assets/payload.txt").read_bytes() == PAYLOAD, "Selected extract payload mismatch")
        full = self.work / "extracted"
        self.cli("extract_entries", {"path": str(self.apk), "output_dir": str(full)})
        repacked = self.work / "repacked.zip"
        self.cli("create_archive", {"source_dir": str(full), "output": str(repacked)})
        require(zip_contents(repacked) == zip_contents(self.apk), "Extract/repack changed file content")
        stored = self.work / "stored.zip"
        self.cli("create_archive", {"source_dir": str(full), "output": str(stored), "store": True})
        require(len(offsets(stored)) == len(zip_contents(stored)), "Store option failed")
        before = self.cli("check_alignment", {"path": str(self.apk)})
        require(before["aligned"] == all(item["aligned"] for item in offsets(self.apk)), "Initial alignment differs from independent ZIP parser")
        aligned = self.work / "aligned.apk"
        self.cli("align_apk", {"path": str(self.apk), "output": str(aligned)})
        after = self.cli("check_alignment", {"path": str(aligned)})
        require(after["aligned"] and all(item["aligned"] for item in offsets(aligned)), "4-byte/16-KiB alignment failed")
        require(zip_contents(aligned) == zip_contents(self.apk), "Alignment changed ZIP contents")

    def xml_resources(self):
        output = self.work / "encoded.axml"
        self.cli("encode_xml", {"path": str(self.xml), "output": str(output)})
        decoded = self.cli("decode_xml", {"path": str(output)})
        root = ET.fromstring(decoded["xml"])
        require(root.attrib["package"] == "engine.library.integration", "XML package not preserved")
        require(root.find("uses-sdk").attrib["{http://schemas.android.com/apk/res/android}minSdkVersion"] == "24", "XML integer attribute mismatch")
        from_apk = self.cli("decode_xml", {"path": str(self.apk), "entry": "AndroidManifest.xml"})
        require(ET.fromstring(from_apk["xml"]).attrib["package"] == "engine.library.integration", "APK manifest decode mismatch")
        raw = self.cli("inspect_resources", {"path": str(self.resources), "offset": 0, "limit": 3})
        packed = self.cli("inspect_resources", {"path": str(self.apk), "entry": "resources.arsc", "offset": 0, "limit": 3})
        require(raw["total"] == 3 and len(raw["entries"]) == 3 and raw["entries"] == packed["entries"], "ARSC raw/APK mismatch")
        require([entry["name"] for entry in raw["entries"]] == ["@string/first", "@string/second", "@string/third"], "ARSC resource names mismatch")
        require([entry["value"] for entry in raw["entries"]] == ["fixture-first", "fixture-second", "fixture-third"], "ARSC string values mismatch")

    def real_reference(self):
        decoded = self.cli("decode_xml", {"path": str(self.reference), "entry": "AndroidManifest.xml"})
        require(ET.fromstring(decoded["xml"]).attrib["package"] == "com.apk.editor", "Real AEE manifest package mismatch")
        packed = self.cli("inspect_resources", {"path": str(self.reference), "entry": "resources.arsc", "offset": 0, "limit": 3})
        raw_path = self.work / "reference-resources.arsc"
        with zipfile.ZipFile(self.reference) as archive:
            raw_path.write_bytes(archive.read("resources.arsc"))
        raw = self.cli("inspect_resources", {"path": str(raw_path), "offset": 0, "limit": 3})
        require(raw["total"] > 100 and len(raw["entries"]) == 3 and raw["entries"] == packed["entries"], "Real AEE ARSC raw/APK mismatch")

    def dex_roundtrip(self):
        first = self.work / "first.dex"
        self.cli("assemble_dex", {"source_dir": str(self.smali), "output": str(first)})
        classes = self.cli("list_dex_classes", {"path": str(first), "prefix": "Lfixture/", "limit": 1})
        require(classes["total"] == 1 and classes["classes"][0]["type"] == "Lfixture/Counter;", "Assembled DEX class missing")
        require(classes["classes"][0]["methods"] == 1 and classes["classes"][0]["fields"] == 1, "DEX member counts mismatch")
        empty = self.cli("list_dex_classes", {"path": str(first), "offset": 9})
        require(empty["classes"] == [], "DEX out-of-range pagination mismatch")
        output = self.work / "disassembled"
        self.cli("disassemble_dex", {"path": str(first), "output_dir": str(output)})
        require(list(output.rglob("*.smali")), "Baksmali wrote no class")
        second = self.work / "second.dex"
        self.cli("assemble_dex", {"source_dir": str(output), "output": str(second)})
        require(first.read_bytes() == second.read_bytes(), "Smali DEX roundtrip changed bytes")
        multi = self.work / "multidex.apk"
        with zipfile.ZipFile(multi, "w") as archive:
            archive.writestr("classes.dex", first.read_bytes())
            archive.writestr("classes2.dex", second.read_bytes())
        selected = self.cli("list_dex_classes", {"path": str(multi), "entry": "classes2.dex"})
        require(selected["dex_entries"] == ["classes2.dex"] and selected["total"] == 1, "Multidex entry selection failed")
        self.cli("disassemble_dex", {"path": str(multi), "output_dir": str(self.work / "multi-error")}, expected=1)
        self.cli("disassemble_dex", {"path": str(multi), "entry": "classes2.dex", "output_dir": str(self.work / "multi-selected")})

    def signing(self):
        output = self.work / "signed.apk"
        signed = self.cli("sign_apk", {"path": str(self.apk), "output": str(output), "keystore": str(self.store), "alias": "fixture", "min_sdk": 24, "v1": False})
        require(signed["verification"]["verified"], "Sign endpoint post-verification failed")
        verified = self.cli("verify_apk", {"path": str(output), "min_sdk": 24, "max_sdk": 35})
        require(verified["verified"] and verified["v2"] and verified["v3"], "v2/v3 signatures failed")
        require([item["sha256"] for item in verified["certificates"]] == [self.certificate_sha256], "Signer certificate identity mismatch")
        data = bytearray(output.read_bytes())
        require(data.count(PAYLOAD) == 1, "Fixture payload must be uniquely stored")
        data[data.index(PAYLOAD)] ^= 1
        tampered = self.work / "tampered.apk"
        tampered.write_bytes(data)
        rejected = self.cli("verify_apk", {"path": str(tampered), "min_sdk": 24, "max_sdk": 35})
        require(not rejected["verified"] and rejected["errors"], "Tampered signature accepted")
        unsigned = self.cli("verify_apk", {"path": str(self.apk), "min_sdk": 24, "max_sdk": 35})
        require(not unsigned["verified"], "Unsigned fixture accepted")

    def error_conditions(self):
        self.process(["unknown-command", "{}"], expected=1)
        self.cli("inspect_apk", {}, expected=1)
        self.cli("inspect_apk", {"path": 3}, expected=1)
        self.cli("inspect_apk", {"path": str(self.apk), "unexpected": True}, expected=1)
        self.cli("inspect_apk", {"path": str(self.work / "absent.apk")}, expected=1)
        for value in [0, 1001, 1.5, "3"]:
            self.cli("list_entries", {"path": str(self.apk), "limit": value}, expected=1)
        self.process(["inspect-apk", "-"], stdin="{broken", expected=1)
        self.process(["inspect-apk", "-"], stdin="[]", expected=1)
        self.process(["inspect-apk", "-"], stdin=json.dumps({"path": str(self.apk)}) + " []", expected=1)
        self.process(["inspect-apk", "-"], stdin="{path:" + json.dumps(str(self.apk)) + "}", expected=1)
        self.cli("align_apk", {"path": str(self.apk), "output": str(self.apk), "overwrite": True}, expected=1)
        occupied = self.work / "occupied.bin"
        occupied.write_bytes(b"keep-existing-output")
        original = digest(occupied)
        self.cli("encode_xml", {"path": str(self.xml), "output": str(occupied)}, expected=1)
        require(digest(occupied) == original, "Rejected overwrite changed existing output")
        self.cli("encode_xml", {"path": str(self.xml), "output": str(occupied), "overwrite": True})
        require(digest(occupied) != original, "Explicit overwrite failed")
        self.cli("create_archive", {"source_dir": str(self.tree), "output": str(self.tree / "bad.zip")}, expected=1)
        require(not (self.tree / "bad.zip").exists(), "Nested output left artifact")
        self.cli("extract_entries", {"path": str(self.apk), "output_dir": str(self.tree)}, expected=1)
        self.cli("extract_entries", {"path": str(self.apk), "output_dir": str(self.work / "missing-extract"), "entries": ["missing"]}, expected=1)
        self.cli("decode_xml", {"path": str(self.apk), "entry": "absent"}, expected=1)
        self.cli("sign_apk", {"path": str(self.apk), "output": str(self.work / "no-password.apk"), "keystore": str(self.store), "alias": "fixture", "password_env": "APK_ENGINE_TEST_UNSET_PASSWORD"}, expected=1)
        require(not (self.work / "no-password.apk").exists(), "Failed signing left output")
        malicious = self.work / "traversal.zip"
        with zipfile.ZipFile(malicious, "w") as archive:
            archive.writestr("../escaped.txt", b"bad")
        self.cli("extract_entries", {"path": str(malicious), "output_dir": str(self.work / "traversal-output")}, expected=1)
        require(not (self.work / "escaped.txt").exists(), "ZIP traversal escaped destination")

    def mcp(self):
        requests = []
        expected = {}

        def rpc(identifier, method, params=None):
            item = {"jsonrpc": "2.0", "method": method}
            if identifier is not None:
                item["id"] = identifier
            if params is not None:
                item["params"] = params
            requests.append(item)

        rpc(1, "tools/list")
        rpc(2, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "independent-verifier", "version": "1"}})
        rpc(None, "notifications/initialized")
        rpc(3, "tools/list")
        rpc(4, "ping")
        operations = [
            ("inspect_apk", {"path": str(self.apk)}),
            ("list_entries", {"path": str(self.apk), "limit": 2}),
            ("extract_entries", {"path": str(self.apk), "output_dir": str(self.work / "mcp-extract"), "entries": ["assets/payload.txt"]}),
            ("create_archive", {"source_dir": str(self.tree), "output": str(self.work / "mcp-created.zip"), "store": True}),
            ("align_apk", {"path": str(self.apk), "output": str(self.work / "mcp-aligned.apk")}),
            ("check_alignment", {"path": str(self.work / "mcp-aligned.apk")}),
            ("list_dex_classes", {"path": str(self.dex), "limit": 1}),
            ("disassemble_dex", {"path": str(self.dex), "output_dir": str(self.work / "mcp-smali")}),
            ("assemble_dex", {"source_dir": str(self.smali), "output": str(self.work / "mcp.dex")}),
            ("encode_xml", {"path": str(self.xml), "output": str(self.work / "mcp.axml")}),
            ("decode_xml", {"path": str(self.work / "mcp.axml")}),
            ("inspect_resources", {"path": str(self.resources), "limit": 2}),
            ("sign_apk", {"path": str(self.apk), "output": str(self.work / "mcp-signed.apk"), "keystore": str(self.store), "alias": "fixture", "min_sdk": 24, "v1": False}),
            ("verify_apk", {"path": str(self.work / "mcp-signed.apk"), "min_sdk": 24, "max_sdk": 35})]
        for identifier, (name, arguments) in enumerate(operations, 10):
            rpc(identifier, "tools/call", {"name": name, "arguments": arguments})
            expected[identifier] = name
        rpc(100, "tools/call", {"name": "inspect_apk", "arguments": {"path": str(self.work / "missing.apk")}})
        rpc(101, "tools/call", {"name": "inspect_apk", "arguments": {"path": 5}})
        rpc(102, "tools/call", {"name": "no_such_tool", "arguments": {}})
        rpc(103, "tools/list", {"cursor": "not-supported"})
        rpc(104, "unknown/method")
        rpc(105, "tools/call", [])
        rpc(106, "tools/call", {"name": "inspect_apk", "arguments": {"path": str(self.apk), "extra": True}})
        rpc(None, "notifications/ignored")
        record = self.process(["--mcp"], stdin="\n".join(json.dumps(request) for request in requests) + "\n{invalid\n")
        responses = [json.loads(line) for line in record["stdout"].splitlines()]
        require(all(item.get("jsonrpc") == "2.0" for item in responses), "MCP stdout contains non-RPC data")
        by_id = {item["id"]: item for item in responses}
        require(len(responses) == len(by_id), "Unexpected duplicate or notification response")
        require(by_id[1]["error"]["code"] == -32602, "MCP lifecycle gate missing")
        require(by_id[2]["result"]["serverInfo"]["name"] == "apk-engine", "Initialize server identity mismatch")
        require(by_id[2]["result"]["protocolVersion"] == "2025-11-25", "Protocol negotiation mismatch")
        require(by_id[3]["result"]["tools"] == self.catalog["tools"], "MCP and CLI catalog differ")
        require(by_id[4]["result"] == {}, "Ping mismatch")
        for identifier, name in expected.items():
            result = by_id[identifier]["result"]
            require(result["isError"] is False, "MCP tool failed: " + name + " " + json.dumps(result))
            structured = result["structuredContent"]
            require(structured["tool"] == name, "MCP tool identity mismatch")
            require(json.loads(result["content"][0]["text"]) == structured, "MCP text and structured content disagree")
            self.mcp_tools.add(name)
        require(by_id[100]["result"]["isError"] is True, "Engine failure lacks isError")
        for identifier in [101, 102, 103, 105, 106]:
            require(by_id[identifier]["error"]["code"] == -32602, "Bad params not distinguished from engine failure")
        require(by_id[104]["error"]["code"] == -32601 and by_id[None]["error"]["code"] == -32700, "JSON-RPC error code mismatch")
        aligned = self.work / "mcp-aligned.apk"
        require(all(item["aligned"] for item in offsets(aligned)), "MCP aligned file fails independent offsets")
        require(zip_contents(self.work / "mcp-created.zip") == zip_contents(self.apk), "MCP repacked archive mismatch")
        require((self.work / "mcp-extract/assets/payload.txt").read_bytes() == PAYLOAD, "MCP extraction mismatch")
        require(by_id[23]["result"]["structuredContent"]["result"]["verified"], "MCP signature verification failed")
        require(self.process(["--mcp"], stdin="")["stdout"] == "", "EOF emitted unexpected MCP data")

    def mcp_invalid_request(self):
        data = self.process(["--mcp"], stdin=json.dumps({"jsonrpc": "2.0", "id": 200}) + "\n")
        response = json.loads(data["stdout"])
        require(response["id"] == 200 and response["error"]["code"] == -32600, "Missing method must produce Invalid Request")

    def unchanged(self):
        for path, expected in self.protected.items():
            require(digest(path) == expected, "Read-only input changed: " + path)
        require(not list(self.work.rglob(".apk-engine-*.part")), "Partial output files remain")


def policy_check():
    scanner_path = Path.home() / ".codex/goal-core/comment-policy/scanner.py"
    rule = Path.home() / ".codex/RULES/no-comments.json"
    if not scanner_path.is_file() or not rule.is_file():
        return {"required_for_current_host": False, "passed": None, "status": "unavailable", "reason": "Canonical managed-agent scanner is external to this portable runtime test harness"}
    sys.dont_write_bytecode = True
    spec = importlib.util.spec_from_file_location("canonical_comment_scanner", scanner_path)
    scanner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(scanner)
    scanner.load_rule(rule)
    path = Path(__file__).resolve()
    result = scanner.scan(path, path.read_text())
    return {"required_for_current_host": True, "policy_path": str(rule), "policy_sha256": digest(rule), "scanner_sha256": digest(scanner_path), "result": result, "passed": not result["findings"] and not result["errors"]}


def main():
    parser = argparse.ArgumentParser()
    target = parser.add_mutually_exclusive_group()
    target.add_argument("--executable")
    target.add_argument("--java-jar", default=str(PROJECT / "dist/apk-engine.jar"))
    parser.add_argument("--java-option", action="append", default=[])
    parser.add_argument("--portable-fixtures", action="store_true")
    parser.add_argument("--reference-apk")
    parser.add_argument("--evidence", default=str(PROJECT / "evidence/integration.json"))
    args = parser.parse_args()
    record = {"schema": "apk-engine.integration/1", "timestamp_utc": datetime.now(timezone.utc).isoformat(), "execution_kind": "native" if args.executable else "jvm", "native_executed": False, "portable_fixtures": True, "real_reference_requested": bool(args.reference_apk), "tester_sha256": digest(__file__)}
    output = Path(args.evidence)
    if output.is_file():
        previous = json.loads(output.read_text())
        history = previous.get("prior_runs", [])
        history.append({"timestamp_utc": previous.get("timestamp_utc"), "target_sha256": previous.get("target_sha256"), "passed": previous.get("passed"), "failed_cases": [{"name": case["name"], "error": case.get("error", "")[:2000]} for case in previous.get("cases", []) if not case["passed"]]})
        record["prior_runs"] = history
    with tempfile.TemporaryDirectory(prefix="apk-engine-integration-") as temporary:
        suite = Suite(args, Path(temporary))
        suite.case("prepare_isolated_fixtures", suite.prepare)
        if suite.cases[-1]["passed"]:
            for name, action in [("catalog_and_schemas", suite.schemas), ("zip_extract_repack_alignment", suite.archives), ("binary_xml_and_arsc", suite.xml_resources), ("smali_dex_roundtrip", suite.dex_roundtrip), ("sign_verify_tamper", suite.signing), ("cli_errors_and_output_preservation", suite.error_conditions), ("mcp_all_tools_and_protocol", suite.mcp), ("mcp_invalid_request", suite.mcp_invalid_request)]:
                suite.case(name, action)
            if args.reference_apk:
                suite.case("real_aee_reference", suite.real_reference)
            suite.case("read_only_inputs_preserved", suite.unchanged)
        record.update({"target": str(suite.target), "target_sha256": suite.protected.get(str(suite.target)), "cases": suite.cases, "cli_coverage": sorted(suite.cli_tools), "mcp_coverage": sorted(suite.mcp_tools), "expected_tools": sorted(TOOLS), "protected_inputs": suite.protected, "calls": suite.calls})
        record["native_executed"] = bool(args.executable and any(call["command"][0] == str(suite.target) for call in suite.calls))
    record["fixtures_keys_and_scratch_removed"] = not Path(temporary).exists()
    try:
        record["comment_policy"] = policy_check()
    except Exception as error:
        record["comment_policy"] = {"passed": False, "error": type(error).__name__ + ": " + str(error)}
    policy_satisfied = record["comment_policy"].get("required_for_current_host") is False or record["comment_policy"]["passed"] is True
    record["passed"] = all(case["passed"] for case in record["cases"]) and set(record["cli_coverage"]) == TOOLS and set(record["mcp_coverage"]) == TOOLS and policy_satisfied and record["fixtures_keys_and_scratch_removed"]
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({"passed": record["passed"], "execution_kind": record["execution_kind"], "cli_tools": len(record["cli_coverage"]), "mcp_tools": len(record["mcp_coverage"]), "evidence": str(output)}, indent=2))
    return 0 if record["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
