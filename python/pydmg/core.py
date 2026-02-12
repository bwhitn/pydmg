from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Dict, List, Optional, Union

from . import _pydmg

PathLike = Union[str, Path]


def _as_path(path: PathLike) -> str:
    return str(Path(path))


def inspect(path: PathLike) -> Dict[str, Any]:
    """Return a full DMG inspection payload.

    The payload includes trailer details, plist metadata, checksum validation,
    partition tables/chunks, and extracted metadata candidates.
    """
    return json.loads(_pydmg.inspect_json(_as_path(path)))


def inspect_filesystems(path: PathLike) -> Dict[str, Any]:
    """Return filesystem metadata discovered in DMG partitions.

    Includes FAT metadata from partition payloads and HFS+/APFS metadata
    via auto-detected Apple filesystem parsing.
    """
    return json.loads(_pydmg.inspect_filesystems_json(_as_path(path)))


def list_partitions(path: PathLike) -> List[Dict[str, Any]]:
    """Return partition metadata and chunk tables for a DMG."""
    return json.loads(_pydmg.list_partitions_json(_as_path(path)))


def inspect_gpt(
    path: PathLike,
    partition_index: Optional[int] = None,
    strict: bool = False,
) -> Dict[str, Any]:
    """Inspect GPT metadata from DMG partition payloads.

    If ``partition_index`` is ``None``, all partitions are scanned and the
    first GPT table found is returned. If none are found, ``has_gpt`` is false.
    Set ``strict=True`` to raise an exception when GPT data is not found.
    """
    return json.loads(_pydmg.inspect_gpt_json(_as_path(path), partition_index, strict))


def read_partition(path: PathLike, index: int) -> bytes:
    """Read and decompress a partition payload by index."""
    return _pydmg.read_partition(_as_path(path), index)


def extract_partition(path: PathLike, index: int, output_path: PathLike) -> int:
    """Write a partition payload to disk and return number of bytes written."""
    return int(_pydmg.extract_partition(_as_path(path), index, _as_path(output_path)))


def compute_data_checksum(path: PathLike) -> int:
    """Compute CRC32 for the DMG data fork."""
    return int(_pydmg.compute_data_checksum(_as_path(path)))


def verify_data_checksum(path: PathLike) -> bool:
    """Compare computed and trailer-declared data fork checksums."""
    return bool(_pydmg.verify_data_checksum(_as_path(path)))


def list_fat32_entries(path: PathLike, partition_index: int = 1) -> List[Dict[str, Any]]:
    """List files/directories inside a FAT32 partition."""
    return json.loads(_pydmg.list_fat32_entries_json(_as_path(path), partition_index))


def extract_fat32(
    path: PathLike,
    output_dir: PathLike,
    partition_index: int = 1,
    overwrite: bool = False,
) -> List[str]:
    """Extract files from a FAT32 partition into ``output_dir``."""
    return json.loads(
        _pydmg.extract_fat32_json(
            _as_path(path),
            _as_path(output_dir),
            partition_index,
            overwrite,
        )
    )


def list_apple_entries(path: PathLike, directory_path: str = "/") -> List[Dict[str, Any]]:
    """List entries from auto-detected HFS+/APFS filesystem."""
    return json.loads(_pydmg.list_apple_entries_json(_as_path(path), directory_path))


def read_apple_file(path: PathLike, file_path: str) -> bytes:
    """Read a file from auto-detected HFS+/APFS filesystem."""
    return _pydmg.read_apple_file(_as_path(path), file_path)


def extract_apple_file(path: PathLike, file_path: str, output_path: PathLike) -> int:
    """Extract a file from auto-detected HFS+/APFS filesystem to disk."""
    return int(_pydmg.extract_apple_file(_as_path(path), file_path, _as_path(output_path)))


def create_dmg(
    source_dir: PathLike,
    output_path: PathLike,
    volume_label: str = "PYDMG",
    total_sectors: int = 32768,
) -> None:
    """Create a DMG containing ``source_dir`` using apple-dmg's FAT32 writer.

    ``total_sectors`` defines total image size in 512-byte sectors.
    """
    _pydmg.create_dmg(_as_path(source_dir), _as_path(output_path), volume_label, total_sectors)


class DmgImage:
    """Convenience object around a DMG file path."""

    def __init__(self, path: PathLike):
        self.path = Path(path)

    def inspect(self) -> Dict[str, Any]:
        return inspect(self.path)

    def inspect_filesystems(self) -> Dict[str, Any]:
        return inspect_filesystems(self.path)

    def partitions(self) -> List[Dict[str, Any]]:
        return list_partitions(self.path)

    def inspect_gpt(
        self,
        partition_index: Optional[int] = None,
        strict: bool = False,
    ) -> Dict[str, Any]:
        return inspect_gpt(self.path, partition_index=partition_index, strict=strict)

    def metadata(self) -> Dict[str, Any]:
        return self.inspect().get("metadata_candidates", {})

    def read_partition(self, index: int) -> bytes:
        return read_partition(self.path, index)

    def extract_partition(self, index: int, output_path: PathLike) -> int:
        return extract_partition(self.path, index, output_path)

    def checksum_valid(self) -> bool:
        return verify_data_checksum(self.path)

    def list_fat32_entries(self, partition_index: int = 1) -> List[Dict[str, Any]]:
        return list_fat32_entries(self.path, partition_index=partition_index)

    def extract_fat32(
        self,
        output_dir: PathLike,
        partition_index: int = 1,
        overwrite: bool = False,
    ) -> List[str]:
        return extract_fat32(
            self.path,
            output_dir=output_dir,
            partition_index=partition_index,
            overwrite=overwrite,
        )

    def list_apple_entries(self, directory_path: str = "/") -> List[Dict[str, Any]]:
        return list_apple_entries(self.path, directory_path=directory_path)

    def read_apple_file(self, file_path: str) -> bytes:
        return read_apple_file(self.path, file_path=file_path)

    def extract_apple_file(self, file_path: str, output_path: PathLike) -> int:
        return extract_apple_file(self.path, file_path=file_path, output_path=output_path)
