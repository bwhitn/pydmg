#!/usr/bin/env python3
"""Prepare structure-aware fuzzing seeds from committed fixtures."""

from __future__ import annotations

import argparse
import bz2
import lzma
import plistlib
import shutil
import struct
import tempfile
import uuid
import zlib
from collections.abc import Iterable
from pathlib import Path

import pydmg

SECTOR_SIZE = 512
MAX_PARTITION_SEED_BYTES = 32 * 1024 * 1024


def write_seed(directory: Path, name: str, data: bytes) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / name).write_bytes(data)


def copy_seed(directory: Path, source: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, directory / source.name)


def iter_blkx_tables(image: Path) -> Iterable[bytes]:
    data = image.read_bytes()
    if len(data) < 512:
        return

    trailer = data[-512:]
    if trailer[:4] != b"koly":
        return

    plist_offset, plist_length = struct.unpack_from(">QQ", trailer, 216)
    plist_end = plist_offset + plist_length
    if plist_end > len(data):
        return

    plist = plistlib.loads(data[plist_offset:plist_end])
    resource_fork = plist.get("resource-fork", plist.get("resourceFork", {}))
    for entry in resource_fork.get("blkx", []):
        table = entry.get("Data")
        if isinstance(table, bytes):
            yield table


def make_chunk_seed(selector: int, payload: bytes, sector_count: int = 1) -> bytes:
    header = bytes([selector])
    header += sector_count.to_bytes(8, "little")
    header += (0).to_bytes(8, "little")
    header += len(payload).to_bytes(8, "little")
    return header + payload


def make_gpt_seed() -> bytes:
    sector_count = 128
    last_lba = sector_count - 1
    entry_count = 128
    entry_size = 128
    table_sectors = (entry_count * entry_size) // SECTOR_SIZE
    backup_table_lba = last_lba - table_sectors
    first_usable_lba = 34
    last_usable_lba = backup_table_lba - 1

    entries = bytearray(entry_count * entry_size)
    entries[0:16] = uuid.UUID("c12a7328-f81f-11d2-ba4b-00a0c93ec93b").bytes_le
    entries[16:32] = uuid.UUID("01234567-89ab-cdef-0123-456789abcdef").bytes_le
    struct.pack_into("<QQQ", entries, 32, first_usable_lba, first_usable_lba + 6, 0)
    name = "FUZZ".encode("utf-16le")
    entries[56 : 56 + len(name)] = name
    entries_crc = zlib.crc32(entries)

    disk_guid = uuid.UUID("00112233-4455-6677-8899-aabbccddeeff").bytes_le

    def header(current_lba: int, alternate_lba: int, entries_lba: int) -> bytes:
        result = bytearray(SECTOR_SIZE)
        struct.pack_into(
            "<8sIIIIQQQQ16sQIII",
            result,
            0,
            b"EFI PART",
            0x00010000,
            92,
            0,
            0,
            current_lba,
            alternate_lba,
            first_usable_lba,
            last_usable_lba,
            disk_guid,
            entries_lba,
            entry_count,
            entry_size,
            entries_crc,
        )
        struct.pack_into("<I", result, 16, zlib.crc32(result[:92]))
        return bytes(result)

    disk = bytearray(sector_count * SECTOR_SIZE)
    protective_size = min(last_lba, 0xFFFF_FFFF)
    struct.pack_into(
        "<B3sB3sII",
        disk,
        446,
        0,
        b"\x00\x02\x00",
        0xEE,
        b"\xff\xff\xff",
        1,
        protective_size,
    )
    disk[510:512] = b"\x55\xaa"
    disk[SECTOR_SIZE : 2 * SECTOR_SIZE] = header(1, last_lba, 2)
    disk[2 * SECTOR_SIZE : 2 * SECTOR_SIZE + len(entries)] = entries
    backup_table_offset = backup_table_lba * SECTOR_SIZE
    disk[backup_table_offset : backup_table_offset + len(entries)] = entries
    disk[last_lba * SECTOR_SIZE :] = header(last_lba, 1, backup_table_lba)
    return bytes(disk)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path(tempfile.gettempdir()) / "pydmg-fuzz-corpus",
    )
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    fixtures = root / "tests" / "fixtures"
    output = args.output_dir.resolve()
    targets = {name: output / name for name in ("dmg_parse", "blkx", "chunk", "gpt", "image")}

    dmg_images = sorted(fixtures.glob("*.dmg"))
    for image in dmg_images:
        copy_seed(targets["dmg_parse"], image)
        copy_seed(targets["image"], image)
        for index, table in enumerate(iter_blkx_tables(image)):
            write_seed(targets["blkx"], f"{image.stem}-{index}.blkx", table)

        try:
            partitions = pydmg.list_partitions(image)
        except RuntimeError:
            partitions = []
        for partition in partitions:
            byte_size = int(partition["table"]["sector_count"]) * SECTOR_SIZE
            if byte_size > MAX_PARTITION_SEED_BYTES:
                continue
            try:
                payload = pydmg.read_partition(image, int(partition["index"]))
            except RuntimeError:
                continue
            write_seed(
                targets["gpt"],
                f"{image.stem}-partition-{partition['index']}.bin",
                payload,
            )

    hfs_fixture = fixtures / "hfsp-small.img.xz"
    write_seed(targets["image"], "hfsp-small.img", lzma.decompress(hfs_fixture.read_bytes()))

    raw_payload = b"pydmg raw chunk seed\n"
    zlib_payload = zlib.compress(b"pydmg zlib chunk seed\n" * 64)
    bzip2_payload = bz2.compress(b"pydmg bzip2 chunk seed\n" * 64)
    write_seed(targets["chunk"], "raw.seed", make_chunk_seed(1, raw_payload))
    write_seed(targets["chunk"], "zlib.seed", make_chunk_seed(5, zlib_payload, 3))
    write_seed(targets["chunk"], "bzip2.seed", make_chunk_seed(6, bzip2_payload, 3))
    write_seed(targets["chunk"], "zero.seed", make_chunk_seed(0, b"", 8))
    write_seed(targets["gpt"], "valid-gpt.bin", make_gpt_seed())

    # Small tokens also help libFuzzer reconstruct nested plist/BLKX structures.
    write_seed(targets["dmg_parse"], "koly.seed", b"koly")
    write_seed(targets["blkx"], "mish.seed", b"mish")
    write_seed(targets["gpt"], "gpt-signature.seed", b"EFI PART")
    write_seed(targets["image"], "empty.seed", b"")

    counts = {name: len(list(directory.iterdir())) for name, directory in targets.items()}
    print(f"fuzz corpus ready: {output}")
    print("seed counts: " + ", ".join(f"{name}={count}" for name, count in counts.items()))
    print("fixture sha256 values are recorded in tests/fixtures/README.md")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
