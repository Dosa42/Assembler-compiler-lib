import json
import os
import sys
import time
from pathlib import Path


FIXTURES = Path(__file__).resolve().parent / "fixtures/monitor"


def main():
    mode = sys.argv[1]
    arguments = sys.argv[2:]
    eof = sys.stdin.buffer.read(1) == b""
    record = {"mode": mode, "arguments": arguments, "pid": os.getpid(), "parent_pid": os.getppid(), "stdin_eof": eof, "path": os.environ.get("PATH"), "java_home": os.environ.get("JAVA_HOME")}
    log = os.environ.get("APK_ENGINE_MONITOR_CALL_LOG")
    if log:
        with Path(log).open("a") as stream:
            stream.write(json.dumps(record) + "\n")
    if not eof or arguments[:4] != ["shell", "dumpsys", "-t", "30"]:
        sys.stderr.write("Unexpected fixture command or open stdin\n")
        return 91
    tail = arguments[4:]
    if tail == ["activity", "services"]:
        name = "modern-services.txt"
    elif tail == ["activity", "lru"]:
        name = "modern-lru.txt"
    elif tail == ["meminfo"]:
        name = "modern-meminfo.txt"
    elif len(tail) == 2 and tail[0] == "meminfo" and tail[1] == "dev.example.meminfo":
        name = "android16.txt"
    else:
        sys.stderr.write("Unexpected fixture arguments: " + json.dumps(tail) + "\n")
        return 92
    if mode == "failure":
        sys.stdout.write("fixture partial stdout\n")
        sys.stderr.write("fixture controlled failure\n")
        return 23
    if mode == "empty":
        return 0
    if mode == "permission":
        sys.stdout.write("Permission Denial: fixture denied dumpsys\n")
        return 0
    if mode == "timeout":
        time.sleep(70)
        return 0
    if mode == "invalid-utf8":
        sys.stdout.buffer.write(b"\xff\xfe")
        return 0
    if mode in ["overflow", "stderr-overflow"]:
        stream = sys.stdout.buffer if mode == "overflow" else sys.stderr.buffer
        for unused in range(257):
            stream.write(b"x" * 65536)
        stream.flush()
        return 0
    if mode == "stderr-heavy":
        sys.stderr.write("e" * 131072)
    else:
        sys.stderr.write(json.dumps(record) + "\n")
    if mode == "malformed":
        sys.stdout.write("unrecognized fixture record\n")
        return 0
    output = (FIXTURES / name).read_text()
    if mode == "batch" and name == "android16.txt":
        output += "\n" + output.replace("pid 4242", "pid 4343").replace("dev.example.meminfo:worker", "dev.example.meminfo:second").replace("2220", "3330").replace("3400", "4400")
    sys.stdout.write(output)
    return 0


if __name__ == "__main__":
    sys.exit(main())
