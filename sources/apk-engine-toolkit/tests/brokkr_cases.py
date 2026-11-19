import hashlib
import json
import os
from pathlib import Path
import select
import struct
import subprocess
import sys
import tarfile
import time

import jsonschema


BROKKR_TOOLS = {"parse_pit", "inspect_firmware", "verify_firmware", "map_firmware_to_pit", "open_usb_session", "inspect_usb_session", "close_usb_session", "probe_device", "flash_firmware"}
USB_TOOLS = {"open_usb_session", "inspect_usb_session", "close_usb_session", "probe_device", "flash_firmware"}


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class BrokkrCases:
    def brokkr_prepare(self):
        library = Path.home() / "engine_library/brokkr"
        engine = Path(self.environment.get("APK_ENGINE_BROKKR", str(library / "bin/brokkr-engine"))).resolve()
        broker = Path(self.environment.get("APK_ENGINE_BROKKR_USB_BROKER", str(library / "usb/bin/brokkr-usb-broker"))).resolve()
        for path in [engine, broker]:
            require(path.is_file() and os.access(path, os.X_OK), "Missing native Brokkr executable: " + str(path))
            self.protected[str(path)] = hashlib.sha256(path.read_bytes()).hexdigest()
        self.environment.update({"APK_ENGINE_BROKKR": str(engine), "APK_ENGINE_BROKKR_USB_BROKER": str(broker)})
        self.brokkr_directory = self.work / "brokkr"
        self.brokkr_directory.mkdir(mode=0o700)
        self.pit = self.brokkr_directory / "fixture.pit"
        entry = struct.pack("<9i", 0, 2, 7, 5, 1, 512, 4, 2048, 0)
        entry += b"BOOT".ljust(32, b"\0") + b"boot.img".ljust(32, b"\0") + b"\0" * 32
        self.pit.write_bytes(struct.pack("<ii8s8sHH", 0x12349876, 1, b"COM_TAR2", b"TESTCPU", 1, 0) + entry)
        self.firmware = self.brokkr_directory / "firmware.tar.md5"
        payload = b"APK-ENGINE-BROKKR-FIXTURE\0\1\2"
        header = tarfile.TarInfo("boot.img")
        header.size = len(payload)
        archive = header.tobuf(format=tarfile.USTAR_FORMAT) + payload + b"\0" * (-len(payload) % 512) + b"\0" * 1024
        self.firmware.write_bytes(archive + hashlib.md5(archive).hexdigest().encode() + b"  firmware.tar\n")
        self.brokkr_payload_bytes = len(payload)
        self.brokkr_archive_bytes = len(archive)
        self.corrupt_firmware = self.brokkr_directory / "corrupt.tar.md5"
        corrupt = bytearray(self.firmware.read_bytes())
        corrupt[512] ^= 1
        self.corrupt_firmware.write_bytes(corrupt)
        self.bad_pit = self.brokkr_directory / "bad.pit"
        self.bad_pit.write_bytes(b"invalid-pit")
        self.usb_fixture = self.brokkr_directory / "regular-file-usb-fixture"
        self.usb_fixture.write_bytes(b"NATIVE-FD-FIXTURE-NOT-A-USB-DEVICE")
        self.usb_socket = self.brokkr_directory / "usb.sock"
        launcher = self.brokkr_directory / "termux-usb-fixture"
        source = Path(__file__).with_name("brokkr_usb_fixture.py")
        launcher.write_text("#!" + str(Path(sys.executable).resolve()) + "\n" + source.read_text())
        launcher.chmod(0o700)
        self.environment["APK_ENGINE_TERMUX_USB"] = str(launcher)
        self.environment["APK_ENGINE_USB_FIXTURE"] = str(self.usb_fixture)
        for path in [self.pit, self.firmware, self.corrupt_firmware, self.bad_pit, self.usb_fixture, source]:
            self.protected[str(path)] = hashlib.sha256(path.read_bytes()).hexdigest()
        self.brokkr_arguments = {
            "parse_pit": {"path": str(self.pit)},
            "inspect_firmware": {"path": str(self.firmware)},
            "verify_firmware": {"path": str(self.firmware)},
            "map_firmware_to_pit": {"pit_path": str(self.pit), "firmware_paths": [str(self.firmware)]},
        }
        self.usb_device_arguments = {"socket": str(self.usb_socket), "usb_interface": 0, "in_endpoint": 129, "out_endpoint": 2}
        self.brokkr_executions = []

    def brokkr_assert(self, name, envelope):
        require(envelope["schema"] == "brokkr.result/1" and envelope["ok"] is True, "Brokkr native envelope mismatch")
        result = envelope["result"]
        if name == "parse_pit":
            require(result["cpu_bl_id"] == "TESTCPU" and len(result["partitions"]) == 1, "PIT header differs from independent fixture")
            partition = result["partitions"][0]
            require(partition["id"] == 7 and partition["name"] == "BOOT" and partition["capacity_bytes"] == 2048, "PIT partition fields differ from fixture")
        elif name == "inspect_firmware":
            require(result["format"] == "tar" and result["entries"] == [{"name": "boot.img", "size_bytes": self.brokkr_payload_bytes, "data_offset": 512}], "Firmware TAR inventory differs from fixture")
            require(len(result["images"]) == 1 and result["images"][0]["basename"] == "boot.img", "Firmware image detection failed")
        elif name == "verify_firmware":
            require(result["structural_valid"] and result["md5_verified"] and result["md5_status"] == "verified" and result["bytes_hashed"] == self.brokkr_archive_bytes, "Firmware checksum verification failed")
        elif name == "map_firmware_to_pit":
            require(len(result["items"]) == 1 and result["items"][0]["partition_id"] == 7 and result["items"][0]["partition_name"] == "BOOT" and result["unmapped"] == [], "Firmware/PIT mapping differs from fixture")

    def brokkr_schemas(self):
        for tool in self.catalog["tools"]:
            if tool["name"] not in BROKKR_TOOLS:
                continue
            jsonschema.Draft202012Validator.check_schema(tool["inputSchema"])
            jsonschema.Draft202012Validator.check_schema(tool["outputSchema"])
            annotations = tool["annotations"]
            require(annotations["destructiveHint"] is (tool["name"] == "flash_firmware"), "Brokkr destructive annotation mismatch")
            require(annotations["readOnlyHint"] is (tool["name"] not in {"open_usb_session", "close_usb_session", "probe_device", "flash_firmware"}), "Brokkr read/write annotation mismatch")
            require(annotations["openWorldHint"] is (tool["name"] in USB_TOOLS), "Brokkr external-device annotation mismatch")

    def brokkr_validate_output(self, name, data):
        descriptor = next(tool for tool in self.catalog["tools"] if tool["name"] == name)
        jsonschema.Draft202012Validator(descriptor["outputSchema"]).validate(data)

    def brokkr_cli(self, name, arguments, expected=0):
        result = self.cli(name, arguments, expected=expected)
        if expected == 0:
            envelope = json.loads(self.calls[-1]["stdout"])
            self.brokkr_validate_output(name, envelope)
        else:
            require("Brokkr " in result["error"]["message"] and "failed with exit" in result["error"]["message"], "Expected native device rejection was not reached")
            require(result["error"]["native_result"]["error"]["code"] == "usb_open" and "libusb_wrap_sys_device" in result["error"]["native_result"]["error"]["message"], "Native device call did not receive fixture descriptor")
            self.cli_tools.add(name)
        self.brokkr_executions.append({"transport": "cli", "tool": name, "expected_native_error": bool(expected)})
        return result

    def brokkr_offline(self):
        self.brokkr_prepare()
        self.brokkr_schemas()
        for name, arguments in self.brokkr_arguments.items():
            self.brokkr_assert(name, self.brokkr_cli(name, arguments))
        invalid = self.cli("verify_firmware", {"path": str(self.corrupt_firmware)}, expected=1)
        require("Brokkr verify-firmware failed" in invalid["error"]["message"], "Corrupt checksum did not reach native verifier")
        invalid = self.cli("parse_pit", {"path": str(self.bad_pit)}, expected=1)
        require("Brokkr parse-pit failed" in invalid["error"]["message"], "Malformed PIT did not reach native parser")
        self.cli("map_firmware_to_pit", {"pit_path": str(self.pit), "firmware_paths": []}, expected=1)
        self.cli("map_firmware_to_pit", {"pit_path": str(self.pit), "firmware_paths": [4]}, expected=1)
        self.cli("probe_device", {"fd": 9}, expected=1)
        self.cli("probe_device", {"socket": "relative"}, expected=1)
        return {"native_companion": self.environment["APK_ENGINE_BROKKR"], "usb_device_operations": False}

    def brokkr_usb_cli(self):
        status = self.brokkr_cli("inspect_usb_session", {"socket": str(self.usb_socket)})
        require(status["state"] == "unavailable", "Initial broker status must be live unavailable")
        status = self.brokkr_cli("open_usb_session", {"device": str(self.usb_fixture), "socket": str(self.usb_socket)})
        require(status["state"] == "open" and status["session"], "Native fixture broker did not open")
        try:
            self.cli("open_usb_session", {"device": str(self.usb_fixture), "socket": str(self.usb_socket)}, expected=1)
            live = self.brokkr_cli("inspect_usb_session", {"socket": str(self.usb_socket)})
            require(live["session"] == status["session"], "Live broker identity changed")
            self.brokkr_cli("probe_device", self.usb_device_arguments, expected=1)
            self.brokkr_cli("flash_firmware", {**self.usb_device_arguments, "firmware_paths": [str(self.firmware)]}, expected=1)
            self.cli("close_usb_session", {"socket": str(self.usb_socket), "session": "wrong-session"}, expected=1)
            require(self.brokkr_cli("inspect_usb_session", {"socket": str(self.usb_socket)})["state"] == "open", "Mismatched token closed broker")
        finally:
            closed = self.brokkr_cli("close_usb_session", {"socket": str(self.usb_socket), "session": status["session"]})
            require(closed["state"] == "closed", "Broker FD closure did not complete")
        require(self.brokkr_cli("inspect_usb_session", {"socket": str(self.usb_socket)})["state"] == "unavailable", "Closed broker remains live")
        return {"descriptor": "real SCM_RIGHTS transfer of regular fixture file", "physical_usb": False, "probe_and_flash": "executed native USB boundary and rejected non-USB descriptor"}

    def brokkr_mcp_usb(self):
        environment = self.environment.copy()
        if self.args.executable:
            environment["PATH"] = "/nonexistent"
            for key in ["JAVA_HOME", "JDK_HOME", "GRAALVM_HOME", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "JDK_JAVA_OPTIONS", "CLASSPATH"]:
                environment.pop(key, None)
        process = subprocess.Popen(self.prefix + ["--mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=environment, cwd=self.work)
        lines = []
        requests = []
        sequence = 0
        started = time.monotonic()
        session = None

        def rpc(method, params=None, notify=False):
            nonlocal sequence
            sequence += 1
            request = {"jsonrpc": "2.0", "method": method}
            if not notify:
                request["id"] = sequence
            if params is not None:
                request["params"] = params
            requests.append(request)
            process.stdin.write(json.dumps(request) + "\n")
            process.stdin.flush()
            if notify:
                return None
            ready, _, _ = select.select([process.stdout], [], [], 45)
            require(bool(ready), "MCP USB fixture response timed out")
            line = process.stdout.readline()
            lines.append(line)
            response = json.loads(line)
            require(response.get("id") == sequence and "result" in response, "MCP USB response mismatch")
            return response["result"]

        def call(name, arguments, expect_error=False):
            result = rpc("tools/call", {"name": name, "arguments": arguments})
            require(result["isError"] is expect_error, "MCP USB result state mismatch: " + name + " " + json.dumps(result))
            data = result["structuredContent"]
            require(json.loads(result["content"][0]["text"]) == data, "MCP USB text/structured content differs")
            if expect_error:
                require("Brokkr " in data["error"]["message"] and "failed with exit" in data["error"]["message"], "MCP USB expected native rejection was not reached")
                require(data["error"]["native_result"]["error"]["code"] == "usb_open" and "libusb_wrap_sys_device" in data["error"]["native_result"]["error"]["message"], "MCP device call did not receive fixture descriptor")
            else:
                self.brokkr_validate_output(name, data)
            self.mcp_tools.add(name)
            self.brokkr_executions.append({"transport": "mcp", "tool": name, "expected_native_error": expect_error})
            return data.get("result", data)

        try:
            rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "brokkr-usb-fixture", "version": "1"}})
            rpc("notifications/initialized", notify=True)
            require(call("inspect_usb_session", {"socket": str(self.usb_socket)})["state"] == "unavailable", "MCP initial broker status mismatch")
            opened = call("open_usb_session", {"device": str(self.usb_fixture), "socket": str(self.usb_socket)})
            require(opened["state"] == "open", "MCP native fixture broker did not open")
            session = opened["session"]
            call("probe_device", self.usb_device_arguments, True)
            call("flash_firmware", {**self.usb_device_arguments, "firmware_paths": [str(self.firmware)]}, True)
            closed = call("close_usb_session", {"socket": str(self.usb_socket), "session": session})
            require(closed["state"] == "closed", "MCP broker did not close")
            session = None
        finally:
            if session:
                self.cli("close_usb_session", {"socket": str(self.usb_socket), "session": session})
            process.stdin.close()
            process.wait(timeout=15)
            stderr = process.stderr.read()
            process.stdout.close()
            process.stderr.close()
            self.calls.append({"command": self.prefix + ["--mcp"], "exit_code": process.returncode, "elapsed_seconds": round(time.monotonic() - started, 3), "stdout": "".join(lines), "stderr": stderr[:8192], "requests": requests, "jvm_hidden_from_child": bool(self.args.executable)})
        require(process.returncode == 0, "MCP USB fixture process failed")
        return {"physical_usb": False, "native_fixture_broker": True, "probe_and_flash": "native rejection of non-USB descriptor"}
