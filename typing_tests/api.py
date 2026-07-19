"""Consumer-style compile-time checks for the complete public API."""

from pathlib import Path
from typing import Any

import pydmg


def check_api() -> None:
    """Type-check calls without executing them as part of the test suite."""
    image_path = Path("example.dmg")
    output_path = Path("output.bin")

    inspection: dict[str, Any] = pydmg.inspect(image_path)
    filesystems: dict[str, Any] = pydmg.inspect_filesystems(image_path)
    gpt: dict[str, Any] = pydmg.inspect_gpt(image_path, partition_index=None, strict=False)
    partitions: list[dict[str, Any]] = pydmg.list_partitions(image_path)
    partition: bytes = pydmg.read_partition(image_path, 0)
    written: int = pydmg.extract_partition(image_path, 0, output_path)
    checksum: int = pydmg.compute_data_checksum(image_path)
    valid: bool = pydmg.verify_data_checksum(image_path)
    fat_entries: list[dict[str, Any]] = pydmg.list_fat32_entries(image_path)
    fat_outputs: list[str] = pydmg.extract_fat32(image_path, Path("fat-output"))
    apple_entries: list[dict[str, Any]] = pydmg.list_apple_entries(image_path)
    apple_file: bytes = pydmg.read_apple_file(image_path, "/README.txt")
    apple_written: int = pydmg.extract_apple_file(image_path, "/README.txt", output_path)
    pydmg.create_dmg(Path("source"), Path("output.dmg"))

    image = pydmg.DmgImage(image_path)
    object_inspection: dict[str, Any] = image.inspect()
    object_filesystems: dict[str, Any] = image.inspect_filesystems()
    object_partitions: list[dict[str, Any]] = image.partitions()
    object_gpt: dict[str, Any] = image.inspect_gpt()
    metadata: dict[str, Any] = image.metadata()
    object_partition: bytes = image.read_partition(0)
    object_written: int = image.extract_partition(0, output_path)
    object_valid: bool = image.checksum_valid()
    object_fat_entries: list[dict[str, Any]] = image.list_fat32_entries()
    object_fat_outputs: list[str] = image.extract_fat32(Path("fat-output"))
    object_apple_entries: list[dict[str, Any]] = image.list_apple_entries()
    object_apple_file: bytes = image.read_apple_file("/README.txt")
    object_apple_written: int = image.extract_apple_file("/README.txt", output_path)

    reveal_values = (
        inspection,
        filesystems,
        gpt,
        partitions,
        partition,
        written,
        checksum,
        valid,
        fat_entries,
        fat_outputs,
        apple_entries,
        apple_file,
        apple_written,
        object_inspection,
        object_filesystems,
        object_partitions,
        object_gpt,
        metadata,
        object_partition,
        object_written,
        object_valid,
        object_fat_entries,
        object_fat_outputs,
        object_apple_entries,
        object_apple_file,
        object_apple_written,
    )
    assert reveal_values
