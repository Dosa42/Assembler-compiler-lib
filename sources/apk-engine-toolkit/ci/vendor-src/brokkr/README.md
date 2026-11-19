# Native Brokkr engine

Objective: reuse Brokkr 2.5.0 firmware parsing, verification, PIT mapping and Odin protocol without the Android UI, JNI, Qt or Java.

The package targets Android Bionic AArch64. `bin/brokkr-engine` calls the same shared C API exposed by `include/brokkr_engine.h` and `lib/libbrokkr-engine.so.1`. Bundled `libusb-1.0.so` and `libc++_shared.so` resolve through relative ELF runpaths; Android supplies libc, libm, libdl, liblog and the dynamic linker. Clear external library overrides when invoking from a shell that injects another C++ runtime:

```sh
env -u LD_LIBRARY_PATH -u LD_PRELOAD ./bin/brokkr-engine parse-pit fixtures/data/sample.pit
env -u LD_LIBRARY_PATH -u LD_PRELOAD ./bin/brokkr-engine inspect-firmware fixtures/data/sample.tar
env -u LD_LIBRARY_PATH -u LD_PRELOAD ./bin/brokkr-engine verify-firmware fixtures/data/sample.tar.md5
env -u LD_LIBRARY_PATH -u LD_PRELOAD ./bin/brokkr-engine map-firmware-to-pit fixtures/data/sample.pit fixtures/data/sample.tar
```

Each invocation emits one JSON object conforming to [result.schema.json](schemas/result.schema.json). Success contains `schema`, `ok`, `operation` and `result`; failure contains `error.code` and `error.message`. Exit status is 0 for success, 1 for operation errors, 2 for argument errors and 70 for an allocation failure. The C API accepts the same arguments without the executable name, allocates the output string and requires `brokkr_free`; calls within one library instance are serialized. [native_client.c](examples/native_client.c) is a separate C consumer.

`inspect-firmware` lists archive entries and the images selected by upstream expansion. `verify-firmware` validates that structure, decompresses selected LZ4 images, validates LZ4 header/content checksums and end markers, and checks an MD5 trailer when present. `md5_status: "absent"` is distinct from a verified checksum; an `.md5` input without a valid trailer fails. These operations do not validate Android boot headers, kernel compatibility, signatures or bootability. Raw images are opaque. PIT capacities use the upstream inferred block layout; mapping does not prove that a device accepts an image. Unmatched images appear in `unmapped`; an entirely empty mapping fails. Upstream duplicate-image selection keeps the later occurrence and honors `meta-data/download-list.txt`.

## USB boundary

The [USB broker](usb/README.md) retains an Android-authorized USB descriptor and transfers a real duplicate over `SCM_RIGHTS`. A descriptor number from another process is insufficient. The native commands accept either `--socket PATH` or a descriptor already inherited by this process:

```sh
./bin/brokkr-engine device-probe --socket PATH
./bin/brokkr-engine flash --socket PATH --ap FILE
./bin/brokkr-engine flash --fd N --interface I --in 0x81 --out 0x02 --ap FILE --reboot
```

Endpoint discovery accepts exactly one alternate-setting-zero interface with one bulk IN and one bulk OUT endpoint. Supply all of `--interface`, `--in` and `--out` to resolve ambiguity. The caller retains ownership of inherited descriptors; the engine closes broker-acquired copies after each call. Callers must serialize operations targeting the same physical device across processes.

`device-probe` performs handshake, protocol negotiation, PIT download and session close without reboot. `flash` uses upstream `odin::flash`, including device PIT mapping, transfers and finalization. Repeat `--ap` for several inputs. A PIT embedded in the inputs follows upstream Android behavior and is uploaded before mapping; `--reboot` requests reboot after successful session finalization. No reboot is the default. Stages and device errors go to stderr. A completed protocol operation does not establish successful boot.

The flash route keeps upstream package MD5 verification and image expansion. It does not impose the explicit verifier's stricter missing-trailer, LZ4 checksum or end-marker checks. Invoke `verify-firmware` separately when those checks are wanted. Upstream LZ4 parsing reads checksum fields without validating them; the stricter implementation lives only in the adapter's verification operation.

## Rebuild and evidence

Run `python build.py` with Termux Clang, CMake, Make, libusb development headers and patchelf installed. The driver uses temporary build space, installs the CLI/shared library here and copies the two redistributable runtime libraries. Run `python verify.py` for independent synthetic PIT/USTAR fixtures, malformed inputs and the external C consumer. `python validate_package.py` validates package records, the comment policy and upstream hashes. Optional private firmware inputs remain outside this package.

For a separate Alpine/musl checkout, `python build.py --static --compiler g++ --c-compiler gcc` builds a static CLI and `lib/libbrokkr-engine.a` using CMake, Make, pkgconf and libusb development/static archives. This route does not copy Android runtime libraries. Its build and execution evidence must be recorded separately from the Bionic package.

The [manifest](engine.json) records source, packaged, installed, reachable and executed states separately. [offline-verification.json](evidence/offline-verification.json) covers offline behavior; [runtime.json](evidence/runtime.json) records ELF linkage and relocated execution. USB protocol code is built but device execution requires its own evidence.

Unchanged upstream sources are retained under `upstream/original` with [provenance hashes](upstream/provenance.json). Adaptations are confined to `src`, the C API header, build/verification drivers and USB broker. They add the JSON/C boundary, argument checks, explicit checksum absence, full selected LZ4 reads, descriptor delivery and endpoint discovery. They leave upstream parser/protocol sources unchanged. Build directories, full Git history, toolchains and private firmware are excluded.

See [THIRD-PARTY.md](licenses/THIRD-PARTY.md) for licenses and dependency source identity.
