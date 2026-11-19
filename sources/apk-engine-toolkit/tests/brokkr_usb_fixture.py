import os
import shlex
import subprocess
import sys


arguments = sys.argv[1:]
if len(arguments) != 5 or arguments[:3] != ["-r", "-E", "-e"]:
    raise SystemExit("Unexpected Termux USB fixture arguments")
fixture = os.environ["APK_ENGINE_USB_FIXTURE"]
if arguments[4] != fixture:
    raise SystemExit("USB fixture path mismatch")
fd = os.open(fixture, os.O_RDONLY)
try:
    command = shlex.split(arguments[3]) + ["--fd", str(fd)]
    result = subprocess.run(command, pass_fds=(fd,), stdin=subprocess.DEVNULL)
finally:
    os.close(fd)
raise SystemExit(result.returncode)
