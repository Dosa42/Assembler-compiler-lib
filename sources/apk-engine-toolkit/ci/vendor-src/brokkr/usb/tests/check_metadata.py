import array
import json
import os
import pathlib
import socket as unix_socket
import subprocess
import tempfile
import jsonschema


root = pathlib.Path(__file__).resolve().parents[1]
schema = json.loads((root / "metadata.schema.json").read_text())
jsonschema.Draft202012Validator.check_schema(schema)
validator = jsonschema.Draft202012Validator(schema)
broker = root / "bin/brokkr-usb-broker"
with tempfile.TemporaryDirectory(prefix="brokkr-json-") as temporary:
    socket = pathlib.Path(temporary) / ("\x01" * (107 - len(os.fsencode(temporary)) - 1))
    assert len(os.fsencode(socket)) == 107
    fixture = pathlib.Path(temporary) / 'fixture-"-\\'
    fixture.write_bytes(b"schema fixture")
    unavailable = subprocess.run([broker, "--socket", socket, "--status"], capture_output=True, text=True)
    assert unavailable.returncode == 1
    missing = json.loads(unavailable.stdout)
    validator.validate(missing)
    assert missing["socket"] == str(socket)
    path_limit = os.pathconf(temporary, "PC_PATH_MAX")
    name_limit = os.pathconf(temporary, "PC_NAME_MAX")
    parent = pathlib.Path(temporary)
    while path_limit - len(os.fsencode(parent)) - 2 > name_limit:
        parent /= "\x01" * name_limit
        parent.mkdir()
    longest = parent / ("\x01" * (path_limit - len(os.fsencode(parent)) - 2))
    longest.write_bytes(b"schema fixture")
    assert len(os.fsencode(longest)) == path_limit - 1
    for fixture in [fixture, longest]:
        with fixture.open("rb") as fd:
            environment = {**os.environ, "TERMUX_USB_FD": str(fd.fileno())}
            process = subprocess.Popen([broker, "--socket", socket], env=environment, pass_fds=(fd.fileno(),), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                line = process.stdout.readline()
                opened = json.loads(line)
                validator.validate(opened)
                assert opened["state"] == "open" and opened["device_path"] == str(fixture)
                assert opened["socket"] == str(socket)
                if fixture == longest:
                    assert len(line.encode()) > 8192
                observed = subprocess.run([broker, "--socket", socket, "--status"], capture_output=True, text=True, check=True)
                live = json.loads(observed.stdout)
                validator.validate(live)
                assert live["device_path"] == str(fixture) and live["socket"] == str(socket)
                with unix_socket.socket(unix_socket.AF_UNIX, unix_socket.SOCK_SEQPACKET) as client:
                    client.connect(str(socket))
                    client.sendall(b"GET\n")
                    data, controls, flags, _ = client.recvmsg(65536, unix_socket.CMSG_SPACE(array.array("i").itemsize))
                    assert not flags & (unix_socket.MSG_TRUNC | unix_socket.MSG_CTRUNC)
                    acquired = json.loads(data)
                    validator.validate(acquired)
                    assert acquired["device_path"] == str(fixture)
                    received = array.array("i")
                    for level, kind, value in controls:
                        if level == unix_socket.SOL_SOCKET and kind == unix_socket.SCM_RIGHTS:
                            received.frombytes(value)
                    try:
                        assert len(received) == 1
                        assert os.read(received[0], 64) == b"schema fixture"
                    finally:
                        for received_fd in received:
                            os.close(received_fd)
                stopped = subprocess.run([broker, "--socket", socket, "--stop", opened["session"]], capture_output=True, text=True, check=True)
                closed = json.loads(stopped.stdout)
                validator.validate(closed)
                assert closed["state"] == "closed" and closed["device_path"] == str(fixture)
                assert process.wait(timeout=5) == 0
                persisted = json.loads(pathlib.Path(str(socket) + ".json").read_text())
                validator.validate(persisted)
                assert persisted["state"] == "closed" and persisted["device_path"] == str(fixture)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
print("PASS broker schema and SCM_RIGHTS: escaped socket107, device PATH_MAX-1, metadata over8192 bytes, open/status/closed/persisted identity")
