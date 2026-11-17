import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess


def inspect(path):
    data = path.read_bytes()
    if data[:6] != b"\x7fELF\x02\x01":
        raise ValueError("Expected little-endian ELF64")
    machine = struct.unpack_from("<H", data, 18)[0]
    if machine != 183:
        raise ValueError("Expected AArch64 ELF")
    phoff = struct.unpack_from("<Q", data, 32)[0]
    phentsize, phnum = struct.unpack_from("<HH", data, 54)
    if phentsize < 56 or phnum == 0 or phoff + phentsize * phnum > len(data):
        raise ValueError("Invalid ELF program headers")
    needed = []
    for index in range(phnum):
        offset = phoff + phentsize * index
        kind = struct.unpack_from("<I", data, offset)[0]
        if kind == 3:
            raise ValueError("PT_INTERP is present")
        if kind == 2:
            segment_offset = struct.unpack_from("<Q", data, offset + 8)[0]
            size = struct.unpack_from("<Q", data, offset + 32)[0]
            if segment_offset + size > len(data) or size % 16:
                raise ValueError("Invalid dynamic segment")
            for position in range(segment_offset, segment_offset + size, 16):
                tag, value = struct.unpack_from("<QQ", data, position)
                if tag == 0:
                    break
                if tag == 1:
                    needed.append(value)
    if needed:
        raise ValueError("DT_NEEDED is present")
    return {"status": "passed", "format": "ELF64 little-endian", "architecture": "AArch64", "pt_interp": False, "dt_needed": [], "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("executable", type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    executable = args.executable.resolve()
    report = inspect(executable)
    result = subprocess.run([str(executable), "tools"], env={"PATH": "/nonexistent", "LANG": "C.UTF-8"}, capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    catalog = json.loads(result.stdout)
    names = [entry["name"] for entry in catalog["tools"]]
    if len(names) != 14:
        raise ValueError("Expected all 14 toolkit tools")
    report.update({"clean_environment_execution": "passed", "child_path": "/nonexistent", "java_home_present": False, "tool_count": len(names), "tool_names": names, "android_device_validation": "pending"})
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    args.evidence.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
