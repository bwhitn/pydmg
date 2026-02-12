# pydmg

`pydmg` is a cross-platform Python library for reading, inspecting, extracting, and creating Apple DMG files.
It uses the Rust crate [`apple-dmg`](https://github.com/indygreg/apple-platform-rs/tree/main/apple-dmg) under the hood and exposes a Pythonic API.

- License: MIT
- Core Rust dependency: `apple-dmg` (`Apache-2.0 OR MIT`, MIT-compatible)
- HFS+/APFS backend: `dpp` (`MIT`)
- Third-party notices: `THIRD_PARTY_NOTICES.md`
- Build backend: `maturin` + `pyo3`
- Platforms: macOS, Linux, Windows
- Distribution: prebuilt wheels for major architectures + source distribution (`sdist`) for any architecture with Rust toolchain

## Features

- Read DMG trailer (`koly`) metadata and checksum fields.
- Parse plist metadata and return a JSON-serializable structure.
- Extract metadata candidates for:
  - creation and modification dates/timestamps
  - author/creator-like fields
  - creation application/tool fields
- List partitions and their BLKX chunk tables.
- Inspect GPT partition-table metadata when GPT is present in DMG partitions.
- Inspect filesystem metadata for FAT12/16/32 partitions.
- Auto-detect and inspect HFS+/APFS metadata using `dpp`.
- Decompress and read partition payload bytes.
- Verify data fork CRC32 checksum.
- List and extract files from FAT32 DMG partitions.
- List and extract files from auto-detected HFS+/APFS DMG filesystems.
- Create new DMGs from a source directory.

## Installation

### From PyPI

```bash
pip install pydmg
```

### From source (local development)

Requirements:

- Python 3.9+
- Rust toolchain (stable)
- C/C++ compiler toolchain

Platform notes for source builds:

- Linux: install `build-essential`, `pkg-config`, `libbz2-dev`, `liblzma-dev`
- macOS: install Xcode Command Line Tools and `pkg-config`
- Windows: install Visual Studio Build Tools (MSVC C++)

```bash
pip install -e ".[dev]"
```

## Quick Start

```python
from pathlib import Path
import pydmg

info = pydmg.inspect("example.dmg")
print(info["koly"]["version"])
print(info["checksum"]["data_fork_checksum_matches"])

parts = pydmg.list_partitions("example.dmg")
print([p["name"] for p in parts])

gpt_info = pydmg.inspect_gpt("example.dmg")
print(gpt_info["has_gpt"])

fat_entries = pydmg.list_fat32_entries("example.dmg", partition_index=1)
print(fat_entries[:5])

pydmg.extract_fat32("example.dmg", "./out", partition_index=1)

fs_info = pydmg.inspect_filesystems("example.dmg")
print(fs_info["fat_filesystems"])

# For HFS+/APFS DMGs:
# entries = pydmg.list_apple_entries("macos_installer.dmg", "/")
# pydmg.extract_apple_file("macos_installer.dmg", "/path/in/image.pkg", "./image.pkg")

pydmg.create_dmg(
    source_dir=Path("./payload"),
    output_path=Path("./payload.dmg"),
    volume_label="PAYLOAD",
    total_sectors=32768,
)
```

## API Surface

### Top-level functions

- `inspect(path)`
- `inspect_filesystems(path)`
- `list_partitions(path)`
- `inspect_gpt(path, partition_index=None, strict=False)`
- `read_partition(path, index)`
- `extract_partition(path, index, output_path)`
- `compute_data_checksum(path)`
- `verify_data_checksum(path)`
- `list_fat32_entries(path, partition_index=1)`
- `extract_fat32(path, output_dir, partition_index=1, overwrite=False)`
- `list_apple_entries(path, directory_path="/")`
- `read_apple_file(path, file_path)`
- `extract_apple_file(path, file_path, output_path)`
- `create_dmg(source_dir, output_path, volume_label="PYDMG", total_sectors=32768)`

### Object API

`DmgImage(path)` provides convenience wrappers over the same operations.

## Function Reference

All paths accept `str` or `pathlib.Path`.

Common error behavior:

- Raises `RuntimeError` when parsing, decompression, or filesystem reads fail.
- Raises `ValueError` for invalid user input in some APIs (for example invalid `create_dmg` arguments).

### DMG inspection and metadata

`inspect(path) -> dict`

- Returns DMG-level details including:
- `koly` trailer values
- `checksum` fields and CRC comparison result
- parsed `plist`
- `metadata_candidates` (date/author/tool-like fields)
- partition metadata and chunk tables

`inspect_filesystems(path) -> dict`

- Returns detected filesystem metadata including:
- `fat_filesystems` list (FAT12/16/32 metadata)
- `apple_filesystem` (HFS+/APFS metadata when detected)
- `errors` list for non-fatal detection failures

`inspect_gpt(path, partition_index=None, strict=False) -> dict`

- Scans one partition or all partitions for GPT metadata.
- If no GPT is found and `strict=False`, returns `has_gpt: false`.
- If no GPT is found and `strict=True`, raises `RuntimeError`.

`list_partitions(path) -> list[dict]`

- Returns parsed DMG partition records with BLKX chunk info.

Example:

```python
import pydmg

info = pydmg.inspect("image.dmg")
print(info["metadata_candidates"])

fs = pydmg.inspect_filesystems("image.dmg")
print(fs["fat_filesystems"])
print(fs["apple_filesystem"])

gpt = pydmg.inspect_gpt("image.dmg", strict=False)
print(gpt["has_gpt"])

parts = pydmg.list_partitions("image.dmg")
print(len(parts))
```

### Partition payload and checksums

`read_partition(path, index) -> bytes`

- Reads and decompresses the partition payload at `index`.

`extract_partition(path, index, output_path) -> int`

- Writes that payload to disk and returns bytes written.

`compute_data_checksum(path) -> int`

- Computes DMG data-fork CRC32.

`verify_data_checksum(path) -> bool`

- Compares computed CRC32 to the trailer-declared checksum.

Example:

```python
payload = pydmg.read_partition("image.dmg", 0)
print(len(payload))

written = pydmg.extract_partition("image.dmg", 0, "part0.bin")
print(written)

print(pydmg.compute_data_checksum("image.dmg"))
print(pydmg.verify_data_checksum("image.dmg"))
```

### FAT32 filesystem helpers

`list_fat32_entries(path, partition_index=1) -> list[dict]`

- Lists FAT entries with path/type/size metadata.

`extract_fat32(path, output_dir, partition_index=1, overwrite=False) -> list[str]`

- Extracts files from the selected FAT partition.
- Returns extracted relative paths.

Example:

```python
entries = pydmg.list_fat32_entries("image.dmg", partition_index=1)
print(entries[:3])

files = pydmg.extract_fat32("image.dmg", "out/fat", partition_index=1, overwrite=True)
print(files[:3])
```

### HFS+ and APFS filesystem helpers

`list_apple_entries(path, directory_path="/") -> list[dict]`

- Auto-detects HFS+ or APFS and lists directory entries.

`read_apple_file(path, file_path) -> bytes`

- Reads a single file from detected HFS+/APFS filesystem.

`extract_apple_file(path, file_path, output_path) -> int`

- Writes file content to `output_path` and returns bytes written.

Example:

```python
entries = pydmg.list_apple_entries("mac_image.dmg", "/")
print(entries[:5])

data = pydmg.read_apple_file("mac_image.dmg", "/README.txt")
print(len(data))

written = pydmg.extract_apple_file("mac_image.dmg", "/README.txt", "out/README.txt")
print(written)
```

### DMG creation

`create_dmg(source_dir, output_path, volume_label="PYDMG", total_sectors=32768) -> None`

- Creates a DMG from a directory.
- `total_sectors` uses 512-byte sectors.

Example:

```python
from pathlib import Path
import pydmg

pydmg.create_dmg(
    source_dir=Path("payload"),
    output_path=Path("payload.dmg"),
    volume_label="PAYLOAD",
    total_sectors=32768,
)
```

### `DmgImage` convenience object

`DmgImage(path)` wraps the same top-level APIs as instance methods.

Example:

```python
img = pydmg.DmgImage("image.dmg")
print(img.inspect()["koly"]["version"])
print(img.inspect_filesystems()["apple_filesystem"])
print(img.checksum_valid())
```

## Testing

```bash
pytest
```

The test suite covers:

- DMG creation from a fixture directory
- inspection payload completeness
- checksum computation/verification
- GPT inspection behavior (non-GPT reporting and strict mode)
- partition read/extract
- FAT32 listing and extraction
- filesystem metadata inspection (FAT + HFS+/APFS auto-detect behavior)
- HFS+ positive listing/read/extract using upstream `hfsplus-rs` fixture image
- APFS positive listing/read/extract using non-malware `linearmouse` fixture image
- partition index error behavior

## API Docs (`pydoc`)

Generate HTML API docs locally:

```bash
python scripts/build_pydoc.py
```

This writes `pydmg.html` in the current directory. CI also verifies that `pydoc`
build succeeds.

## GitHub Actions and PyPI

This repo includes workflows for:

- CI: build + test on Linux, macOS, and Windows
- Release: build wheels for major architectures, build source distribution, and publish to PyPI

Release workflow targets:

- Linux: `x86_64`, `aarch64`
- macOS: `x86_64`, `arm64`
- Windows: `x86_64`
- Plus `sdist` for architecture-independent source release

For publishing, configure PyPI trusted publishing for this repository and push a tag like `v0.1.0`.

Detailed release steps are documented in `RELEASE.md`.

## Licensing Notes

- Project license: MIT (`LICENSE`)
- Third-party attribution and fixture provenance: `THIRD_PARTY_NOTICES.md`
- Test fixture source details: `tests/fixtures/README.md`
- No Paragon APFS SDK code is included

## Notes on metadata completeness

DMG metadata fields vary by producer and format. `pydmg` returns:

- parsed plist payload (JSON-compatible representation)
- extracted metadata candidates based on key patterns

This gives broad coverage of creation dates, creators/authors, and creation applications when present.
