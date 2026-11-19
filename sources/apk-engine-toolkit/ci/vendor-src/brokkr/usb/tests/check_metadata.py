import json
import os
import pathlib
import subprocess
import tempfile
import jsonschema


root = pathlib.Path(__file__).resolve().parents[1]
schema = json.loads((root / "metadata.schema.json").read_text())
jsonschema.Draft202012Validator.check_schema(schema)
validator = jsonschema.Draft202012Validator(schema)
broker = root / "bin/brokkr-usb-broker"
with tempfile.TemporaryDirectory(prefix="brokkr-json-") as temporary:
    socket = pathlib.Path(temporary) / "socket"
    fixture = pathlib.Path(temporary) / 'fixture-"-\\'
    fixture.write_bytes(b"schema fixture")
    unavailable = subprocess.run([broker, "--socket", socket, "--status"], capture_output=True, text=True)
    assert unavailable.returncode == 1
    validator.validate(json.loads(unavailable.stdout))
    with fixture.open("rb") as fd:
        environment = {**os.environ, "TERMUX_USB_FD": str(fd.fileno())}
        process = subprocess.Popen([broker, "--socket", socket], env=environment, pass_fds=(fd.fileno(),), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            opened = json.loads(process.stdout.readline())
            validator.validate(opened)
            assert opened["state"] == "open" and opened["device_path"] == str(fixture)
            observed = subprocess.run([broker, "--socket", socket, "--status"], capture_output=True, text=True, check=True)
            validator.validate(json.loads(observed.stdout))
            stopped = subprocess.run([broker, "--socket", socket, "--stop", opened["session"]], capture_output=True, text=True, check=True)
            validator.validate(json.loads(stopped.stdout))
            assert json.loads(stopped.stdout)["state"] == "closed"
            assert process.wait(timeout=5) == 0
            persisted = json.loads(pathlib.Path(str(socket) + ".json").read_text())
            validator.validate(persisted)
            assert persisted["state"] == "closed"
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
print("PASS broker consumer schema: live open/status/closed, unavailable, persisted closed, escaped descriptor path")
