from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Union, cast

from . import _pydmg

PathLike = Union[str, Path]


def _as_path(path: PathLike) -> str:
    return str(Path(path))


def inspect(path: PathLike) -> dict[str, Any]:
    """Return a full DMG inspection payload.

    The payload includes trailer details, plist metadata, checksum validation,
    partition tables/chunks, and extracted metadata candidates.

    Malformed ranges, excessive plist/BLKX structures, and oversized expanded
    partitions raise ``RuntimeError`` under the limits documented in SECURITY.md.
    """
    return cast(dict[str, Any], json.loads(_pydmg.inspect_json(_as_path(path))))


def inspect_filesystems(path: PathLike) -> dict[str, Any]:
    """Return filesystem metadata discovered in DMG partitions.

    Includes FAT metadata from partition payloads and HFS+/APFS metadata
    via auto-detected Apple filesystem parsing.

    Dependency panics are contained as ``RuntimeError`` or a non-fatal item in
    the returned ``errors`` list, although the Rust panic hook may write stderr.
    """
    return cast(dict[str, Any], json.loads(_pydmg.inspect_filesystems_json(_as_path(path))))


def list_partitions(path: PathLike) -> list[dict[str, Any]]:
    """Return partition metadata and chunk tables for a DMG."""
    return cast(list[dict[str, Any]], json.loads(_pydmg.list_partitions_json(_as_path(path))))


def inspect_gpt(
    path: PathLike,
    partition_index: int | None = None,
    strict: bool = False,
) -> dict[str, Any]:
    """Inspect GPT metadata from DMG partition payloads.

    If ``partition_index`` is ``None``, all partitions are scanned and the
    first GPT table found is returned. If none are found, ``has_gpt`` is false.
    Set ``strict=True`` to raise an exception when GPT data is not found.
    """
    return cast(
        dict[str, Any],
        json.loads(_pydmg.inspect_gpt_json(_as_path(path), partition_index, strict)),
    )


def read_partition(path: PathLike, index: int) -> bytes:
    """Read and decompress a partition payload by index, up to 512 MiB.

    ADC compression is unsupported upstream. An invalid index raises
    ``IndexError``; malformed or over-limit content raises ``RuntimeError``.
    """
    return _pydmg.read_partition(_as_path(path), index)


def extract_partition(path: PathLike, index: int, output_path: PathLike) -> int:
    """Atomically write a bounded partition payload and return its byte count.

    The final path is replaced only after success. A final symlink is replaced
    rather than followed, and failure preserves the previous destination.
    """
    return int(_pydmg.extract_partition(_as_path(path), index, _as_path(output_path)))


def compute_data_checksum(path: PathLike) -> int:
    """Compute CRC32 for a validated DMG data-fork range.

    CRC32 detects accidental corruption; it is not authenticity verification.
    """
    return int(_pydmg.compute_data_checksum(_as_path(path)))


def verify_data_checksum(path: PathLike) -> bool:
    """Compare computed and declared CRC32 after validating the data-fork range.

    A true result is not proof of publisher identity or hostile-tamper resistance.
    """
    return bool(_pydmg.verify_data_checksum(_as_path(path)))


def list_fat32_entries(path: PathLike, partition_index: int = 1) -> list[dict[str, Any]]:
    """List a FAT partition with entry, depth, byte, and read-operation limits."""
    return cast(
        list[dict[str, Any]],
        json.loads(_pydmg.list_fat32_entries_json(_as_path(path), partition_index)),
    )


def extract_fat32(
    path: PathLike,
    output_dir: PathLike,
    partition_index: int = 1,
    overwrite: bool = False,
) -> list[str]:
    """Extract a bounded FAT partition into a no-symlink output tree.

    This separate bulk API writes image-derived names. The function rejects
    symlink roots, components, and file destinations, honors ``overwrite``,
    and writes each file through a sibling temporary file before atomic
    persistence. The caller must control ``output_dir``.
    """
    return cast(
        list[str],
        json.loads(
            _pydmg.extract_fat32_json(
                _as_path(path),
                _as_path(output_dir),
                partition_index,
                overwrite,
            )
        ),
    )


def list_apple_entries(path: PathLike, directory_path: str = "/") -> list[dict[str, Any]]:
    """List a bounded directory from an auto-detected HFS+/APFS filesystem.

    Upstream parser panics are contained as ``RuntimeError`` but may emit a
    panic-hook diagnostic to stderr.
    """
    return cast(
        list[dict[str, Any]],
        json.loads(_pydmg.list_apple_entries_json(_as_path(path), directory_path)),
    )


def read_apple_file(path: PathLike, file_path: str) -> bytes:
    """Read up to 512 MiB from an auto-detected HFS+/APFS filesystem."""
    return _pydmg.read_apple_file(_as_path(path), file_path)


def extract_apple_file(path: PathLike, file_path: str, output_path: PathLike) -> int:
    """Atomically extract a bounded HFS+/APFS file to ``output_path``.

    Failure preserves the previous output, and a final symlink is replaced
    instead of followed.
    """
    return int(_pydmg.extract_apple_file(_as_path(path), file_path, _as_path(output_path)))


def create_dmg(
    source_dir: PathLike,
    output_path: PathLike,
    volume_label: str = "PYDMG",
    total_sectors: int = 32768,
) -> None:
    """Create a DMG containing ``source_dir`` using apple-dmg's FAT32 writer.

    ``total_sectors`` defines total image size in 512-byte sectors and may not
    exceed 1,048,576 (512 MiB). The final output is replaced atomically only
    after successful creation. The caller-controlled source tree is assumed
    to remain immutable for the duration of the operation.
    """
    _pydmg.create_dmg(_as_path(source_dir), _as_path(output_path), volume_label, total_sectors)


class DmgImage:
    """Convenience object around a DMG file path."""

    def __init__(self, path: PathLike):
        """Bind the convenience object to ``path`` without opening it yet."""
        self.path = Path(path)

    def inspect(self) -> dict[str, Any]:
        """Return the full inspection payload for this image."""
        return inspect(self.path)

    def inspect_filesystems(self) -> dict[str, Any]:
        """Return detected FAT, HFS+, and APFS metadata."""
        return inspect_filesystems(self.path)

    def partitions(self) -> list[dict[str, Any]]:
        """Return partition and BLKX metadata."""
        return list_partitions(self.path)

    def inspect_gpt(
        self,
        partition_index: int | None = None,
        strict: bool = False,
    ) -> dict[str, Any]:
        """Inspect GPT data, optionally within one partition."""
        return inspect_gpt(self.path, partition_index=partition_index, strict=strict)

    def metadata(self) -> dict[str, Any]:
        """Return date, author, and creation-application candidates."""
        return cast(dict[str, Any], self.inspect().get("metadata_candidates", {}))

    def read_partition(self, index: int) -> bytes:
        """Read and decompress partition ``index`` with the configured size limit."""
        return read_partition(self.path, index)

    def extract_partition(self, index: int, output_path: PathLike) -> int:
        """Atomically extract partition ``index`` to ``output_path``."""
        return extract_partition(self.path, index, output_path)

    def checksum_valid(self) -> bool:
        """Return whether the data-fork CRC32 matches the trailer value."""
        return verify_data_checksum(self.path)

    def list_fat32_entries(self, partition_index: int = 1) -> list[dict[str, Any]]:
        """List entries in a FAT partition."""
        return list_fat32_entries(self.path, partition_index=partition_index)

    def extract_fat32(
        self,
        output_dir: PathLike,
        partition_index: int = 1,
        overwrite: bool = False,
    ) -> list[str]:
        """Safely extract a FAT partition beneath ``output_dir``."""
        return extract_fat32(
            self.path,
            output_dir=output_dir,
            partition_index=partition_index,
            overwrite=overwrite,
        )

    def list_apple_entries(self, directory_path: str = "/") -> list[dict[str, Any]]:
        """List one HFS+ or APFS directory."""
        return list_apple_entries(self.path, directory_path=directory_path)

    def read_apple_file(self, file_path: str) -> bytes:
        """Read a bounded file from an HFS+ or APFS image."""
        return read_apple_file(self.path, file_path=file_path)

    def extract_apple_file(self, file_path: str, output_path: PathLike) -> int:
        """Atomically extract a bounded HFS+ or APFS file."""
        return extract_apple_file(self.path, file_path=file_path, output_path=output_path)
