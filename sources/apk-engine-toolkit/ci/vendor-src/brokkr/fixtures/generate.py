import hashlib
import struct
import tarfile
from pathlib import Path


def pit_entry(identifier, name, filename, begin, count, dev_type=2):
    entry = bytearray(132)
    struct.pack_into("<9i", entry, 0, 0, dev_type, identifier, 5, 1, 512, count, begin, 0)
    entry[36:68] = name.encode().ljust(32, b"\0")
    entry[68:100] = filename.encode().ljust(32, b"\0")
    return bytes(entry)


def tar_member(name, data):
    header = tarfile.TarInfo(name)
    header.size = len(data)
    header.mode = 0o644
    header.mtime = 0
    return header.tobuf(format=tarfile.USTAR_FORMAT) + data + b"\0" * (-len(data) % 512)


def generate(destination):
    destination = Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    boot = b"BROKKR-OFFLINE-BOOT\x00\x01\x02\x03"
    vbmeta = b"BROKKR-VBMETA\x00\x01\x02"
    boot_lz4 = bytes.fromhex("04224d186c40b800000000000000c822000000ff0842524f4b4b522d4f46464c494e452d424f4f540001020317008950540001020300000000953dfb31")
    corrupt_lz4 = bytearray(boot_lz4)
    corrupt_lz4[44:46] = b"\0\0"
    bad_lz4_header = bytearray(boot_lz4)
    bad_lz4_header[14] ^= 1
    header = struct.pack("<ii8s8sHH", 0x12349876, 2, b"COM_TAR2", b"TESTCPU", 1, 0)
    pit = header + pit_entry(7, "BOOT", "boot.img", 2048, 4)
    pit += pit_entry(9, "VBMETA", "vbmeta.img", 4096, 8)
    archive = tar_member("images/boot.img", boot) + tar_member("vbmeta.img", vbmeta) + b"\0" * 1024
    digest = hashlib.md5(archive).hexdigest().encode()
    trailer = digest + b"  fixture.tar\n"
    corrupt_payload = bytearray(archive + trailer)
    corrupt_payload[512] ^= 1
    bad_checksum = bytearray(archive)
    bad_checksum[0] ^= 1
    negative_count = bytearray(pit)
    struct.pack_into("<i", negative_count, 4, -1)
    inflated_count = bytearray(pit)
    struct.pack_into("<i", inflated_count, 4, 1000)
    invalid_magic = bytearray(pit)
    struct.pack_into("<I", invalid_magic, 0, 0)
    ab_header = struct.pack("<ii8s8sHH", 0x12349876, 2, b"COM_TAR2", b"TESTUFS", 2, 0)
    ab_pit = ab_header + pit_entry(11, "VBMETA_A", "vbmeta_a.img", 1024, 4, 8)
    ab_pit += pit_entry(12, "VBMETA_B", "vbmeta_b.img", 1024, 8, 2)
    fixtures = {
        "sample.pit": pit,
        "mixed-ab.pit": ab_pit,
        "boot.img": boot,
        "compressed.img.lz4": boot_lz4,
        "corrupt-compressed.img.lz4": bytes(corrupt_lz4),
        "bad-header.img.lz4": bytes(bad_lz4_header),
        "missing-footer.img.lz4": boot_lz4[:-4],
        "vbmeta.img": vbmeta,
        "unknown.img": b"unmapped-image",
        "fixture.tar": archive,
        "fixture.tar.md5": archive + trailer,
        "corrupt-payload.tar.md5": bytes(corrupt_payload),
        "bad-checksum.tar": bytes(bad_checksum),
        "truncated.tar": archive[:520],
        "missing-trailer.tar.md5": archive,
        "bad-magic.pit": bytes(invalid_magic),
        "negative-count.pit": bytes(negative_count),
        "inflated-count.pit": bytes(inflated_count),
        "short-header.pit": pit[:27],
        "short-entry.pit": pit[:-1],
    }
    for name, data in fixtures.items():
        (destination / name).write_bytes(data)
    return {
        "archive_bytes": 3072,
        "md5": digest.decode(),
        "boot_bytes": len(boot),
        "vbmeta_bytes": len(vbmeta),
        "compressed_bytes": 184,
        "compressed_disk_bytes": 61,
        "files": {name: hashlib.sha256(data).hexdigest() for name, data in fixtures.items()},
    }


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent / "data")
