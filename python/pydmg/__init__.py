from __future__ import annotations

from importlib.metadata import PackageNotFoundError, version

from .core import (
    DmgImage,
    compute_data_checksum,
    create_dmg,
    extract_apple_file,
    extract_fat32,
    extract_partition,
    inspect,
    inspect_filesystems,
    inspect_gpt,
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
    "__version__",
    "compute_data_checksum",
    "create_dmg",
    "extract_apple_file",
    "extract_fat32",
    "extract_partition",
    "inspect",
    "inspect_filesystems",
    "inspect_gpt",
    "list_apple_entries",
    "list_fat32_entries",
    "list_partitions",
    "read_apple_file",
    "read_partition",
    "verify_data_checksum",
]
