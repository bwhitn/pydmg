from __future__ import annotations

from importlib.metadata import PackageNotFoundError, version

from .core import (
    DmgImage,
    compute_data_checksum,
    create_dmg,
    extract_apple_file,
    extract_fat32,
    extract_partition,
    inspect_filesystems,
    inspect_gpt,
    inspect,
    list_apple_entries,
    list_fat32_entries,
    list_partitions,
    read_apple_file,
    read_partition,
    verify_data_checksum,
)

try:
    __version__ = version("pydmg")
except PackageNotFoundError:
    __version__ = "0.0.0"

__all__ = [
    "DmgImage",
    "inspect",
    "inspect_filesystems",
    "inspect_gpt",
    "list_partitions",
    "read_partition",
    "extract_partition",
    "list_fat32_entries",
    "extract_fat32",
    "list_apple_entries",
    "read_apple_file",
    "extract_apple_file",
    "create_dmg",
    "compute_data_checksum",
    "verify_data_checksum",
    "__version__",
]
