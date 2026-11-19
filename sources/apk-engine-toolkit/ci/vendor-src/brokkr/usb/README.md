# Brokkr USB descriptor bridge

Objective: Retain an Android-authorized USB descriptor in a native broker and transfer real duplicates to separately launched native Brokkr commands.

## Build and launch

Run `make` in this directory. The broker and client use Android libc; neither requires Python or a JVM. Python and `jsonschema` are used only by the metadata test.

```sh
termux-usb -l
termux-usb -r -E -e /data/data/com.termux/files/home/engine_library/brokkr/usb/bin/brokkr-usb-broker DEVICE
```

Replace `DEVICE` with the current listed path. Android handles its existing permission grant or permission dialog. The broker remains in the foreground for the session. Its default socket is `/data/data/com.termux/files/usr/tmp/brokkr-usb/socket`; no environment setting is needed to find it.

For a manually inherited descriptor, use `bin/brokkr-usb-broker --socket /private/directory/socket --fd FD`. The directory must belong to the current UID and deny group/other access. A bare descriptor argument and `TERMUX_USB_FD` are supported. `BROKKR_USB_SOCKET` is an optional fallback; `--socket` overrides it.

The installed `termux-usb` 0.60.0 script exports `TERMUX_CALLBACK` and `TERMUX_EXPORT_FD`; its help specifies `TERMUX_USB_FD` for `-E`. The installed `/data/data/com.termux/files/usr/libexec/termux-callback` runs `$TERMUX_CALLBACK "$@"`. Simple callback arguments split on whitespace; embedded quote syntax is not reparsed and wildcard expansion can occur. Use the default path or a literal callback path without whitespace or shell wildcard characters. These are inspected script contracts; a real Android USB callback was not executed by the fixture tests.

## Observe and close

```sh
bin/brokkr-usb-broker --status
bin/brokkr-usb-broker --stop SESSION
```

Both commands accept `--socket PATH`. `--status` performs a live RPC. Success returns exit code 0 and metadata conforming to [metadata.schema.json](metadata.schema.json). Missing or unreachable sockets return exit code 1 and `state: unavailable`. A session mismatch returns exit code 1 with the live instance metadata and `error: session_mismatch`.

`state: open` means the broker retains a valid descriptor. It does not establish that the USB device is still connected, that an interface has been claimed, or that an Odin session has started. Vendor/product IDs come from readable matching sysfs entries and are `null` when unavailable. The native consumer inspects USB interfaces and endpoints after acquiring its descriptor.

The launcher can report waiting for Android permission only while its own request remains pending. No broker exists before the callback receives a descriptor. `PATH.json` is a timestamped snapshot, written at open and orderly close; it cannot establish liveness. Abrupt termination can leave an `open` snapshot and stale socket. The next broker recovers a refused, same-UID socket under its exclusive lock. A live broker is never replaced.

The 32-character `session` value identifies one broker instance. `--stop SESSION` closes only the matching broker descriptor and ends that broker. SIGTERM and SIGINT also close it. Descriptors already transferred to clients remain owned by those clients until they close them; shutdown cannot revoke those copies. No PID-based kill is used by the stop command.

## Native client and wire contract

Compile [brokkr_usb_client.c](brokkr_usb_client.c) and include [brokkr_usb_client.h](brokkr_usb_client.h). `brokkr_usb_acquire(socket, json, capacity)` returns a caller-owned descriptor with `FD_CLOEXEC`, or `-1` with `errno`. Close it after the USB operation. Status and token-based stop helpers return 0 on success or -1 with `errno`. A received error response remains available in the JSON buffer.

The Unix socket uses `SOCK_SEQPACKET`. Each connection sends one ASCII packet: `GET\n`, `STATUS\n`, or `STOP <session>\n`. The broker returns one UTF-8 JSON packet. Only a successful GET attaches one `SOL_SOCKET`/`SCM_RIGHTS` descriptor. Both sides verify the peer UID. RPC send/receive timeouts are three seconds; the broker retains its own descriptor across client requests. The bridge does not serialize client USB operations.

## Verification and provenance

`make check` launches the actual ARM64 broker with a temporary regular-file descriptor and verifies `SCM_RIGHTS` readback, descriptor lifetime, ownership tokens and cleanup. `python tests/check_metadata.py` validates emitted records against the broker schema, including escaped descriptor paths. Neither test requests USB permission or accesses a physical USB device.

The bridge source, header and tests were authored for this task. They contain no copied upstream Brokkr implementation. [MIT licensing](LICENSE) permits integration with the GPL-licensed companion while preserving this notice. The local binary is built from these sources; app packaging, installation and physical-device execution are separate integration states.
