import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import platform

ROOT = Path(__file__).resolve().parent
PREFIX = Path(os.environ.get("PREFIX", "/data/data/com.termux/files/usr"))
ENV = {key: value for key, value in os.environ.items() if key not in ("LD_LIBRARY_PATH", "LD_PRELOAD")}


def run(argv):
    subprocess.run(argv, check=True, env=ENV)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--static", action="store_true")
    parser.add_argument("--compiler")
    parser.add_argument("--c-compiler")
    args = parser.parse_args()
    configure = ["-DBROKKR_STATIC=" + ("ON" if args.static else "OFF")]
    if args.compiler:
        configure.append("-DCMAKE_CXX_COMPILER=" + args.compiler)
    if args.c_compiler:
        configure.append("-DCMAKE_C_COMPILER=" + args.c_compiler)
    with tempfile.TemporaryDirectory(prefix="brokkr-build-") as directory:
        run(["cmake", "-S", str(ROOT), "-B", directory, "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_INSTALL_PREFIX=" + str(ROOT), *configure])
        run(["cmake", "--build", directory, "--parallel", "3"])
        run(["cmake", "--install", directory])
    if not args.static:
        run(["patchelf", "--set-rpath", "$ORIGIN/../lib", str(ROOT / "bin/brokkr-engine")])
        run(["patchelf", "--set-rpath", "$ORIGIN", str(ROOT / "lib/libbrokkr-engine.so.1")])
        for name in ("libusb-1.0.so", "libc++_shared.so"):
            shutil.copy2(PREFIX / "lib" / name, ROOT / "lib" / name)
            run(["patchelf", "--set-rpath", "$ORIGIN", str(ROOT / "lib" / name)])
        for package, name in (("libusb", "libusb-LGPL-2.1.txt"), ("libc++", "libcxx-Termux-copyright.txt")):
            shutil.copy2(PREFIX / "share/doc" / package / "copyright", ROOT / "licenses" / name)
    artifacts = []
    for directory in ("bin", "lib"):
        for path in sorted((ROOT / directory).iterdir()):
            if path.is_file():
                artifacts.append({"path": str(path.relative_to(ROOT)), "bytes": path.stat().st_size,
                                  "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    compiler = args.compiler or os.environ.get("CXX", "c++")
    dependencies = (["libusb " + subprocess.check_output(["pkg-config", "--modversion", "libusb-1.0"], text=True, env=ENV).strip()] if args.static else
                    subprocess.check_output(["dpkg-query", "-W", "-f=${Package} ${Version}\n", "libusb", "libc++"], text=True).splitlines())
    evidence = {"schema": "brokkr.build/1", "producer": "brokkr/build.py", "target": ("Linux static " if args.static else "Android Bionic ") + platform.machine(),
                "compiler": subprocess.check_output([compiler, "--version"], text=True, env=ENV).splitlines()[0],
                "dependencies": dependencies,
                "artifacts": artifacts}
    (ROOT / "evidence").mkdir(exist_ok=True)
    (ROOT / "evidence/build.json").write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
