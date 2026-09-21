from __future__ import annotations

import lzma
from pathlib import Path

import pydmg
import pytest


def _build_source_tree(tmp_path: Path) -> Path:
    source = tmp_path / "payload"
    source.mkdir()
    (source / "hello.txt").write_text("hello from pydmg\n", encoding="utf-8")
    docs = source / "docs"
    docs.mkdir()
    (docs / "readme.md").write_text("sample file\n", encoding="utf-8")
    return source


def _extract_fixture_xz(tmp_path: Path, fixture_name: str) -> Path:
    fixture_path = Path(__file__).parent / "fixtures" / fixture_name
    output = tmp_path / fixture_name.removesuffix(".xz")
    output.write_bytes(lzma.decompress(fixture_path.read_bytes()))
    return output


def _build_dmg(tmp_path: Path) -> tuple[Path, Path]:
    source = _build_source_tree(tmp_path)
    dmg = tmp_path / "sample.dmg"
    pydmg.create_dmg(source, dmg, volume_label="TESTVOL", total_sectors=32768)
    assert dmg.exists()
    assert dmg.stat().st_size > 0
    return source, dmg


def _symlink_or_skip(link: Path, target: Path, *, target_is_directory: bool = False) -> None:
    try:
        link.symlink_to(target, target_is_directory=target_is_directory)
    except (NotImplementedError, OSError) as error:
        pytest.skip(f"symlinks are unavailable in this environment: {error}")


def _find_first_regular_file(image_path: Path) -> str | None:
    queue = ["/"]
    visited = set(queue)
    scanned_dirs = 0
    max_dirs = 512

    while queue and scanned_dirs < max_dirs:
        directory = queue.pop(0)
        scanned_dirs += 1
        entries = pydmg.list_apple_entries(image_path, directory_path=directory)

        for entry in entries:
            path = entry["path"]
            if entry["is_dir"]:
                if path not in visited:
                    visited.add(path)
                    queue.append(path)
            else:
                return path

    return None


def test_inspect_includes_core_sections(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)

    info = pydmg.inspect(dmg)

    assert info["koly"]["version"] == 4
    assert info["checksum"]["data_fork_checksum_matches"] is True
    assert isinstance(info["plist"], dict)
    assert isinstance(info["metadata_candidates"], dict)
    assert len(info["partitions"]) >= 2
    assert isinstance(info["fat_filesystems"], list)
    assert "apple_filesystem" in info
    assert isinstance(info["filesystem_errors"], list)


def test_inspect_gpt_non_strict_reports_absence(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)

    info = pydmg.inspect_gpt(dmg)

    assert info["has_gpt"] is False
    assert info["partition_index"] is None
    assert isinstance(info["errors"], list)
    assert info["errors"] == []


def test_inspect_gpt_strict_raises(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)

    with pytest.raises(RuntimeError):
        pydmg.inspect_gpt(dmg, strict=True)


def test_partition_read_and_extract(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)

    partition_bytes = pydmg.read_partition(dmg, 1)
    assert isinstance(partition_bytes, (bytes, bytearray))
    assert len(partition_bytes) > 0

    partition_file = tmp_path / "fat32_partition.bin"
    written = pydmg.extract_partition(dmg, 1, partition_file)
    assert written == len(partition_bytes)
    assert partition_file.read_bytes() == partition_bytes

    nested_partition_file = tmp_path / "nested" / "dir" / "partition.bin"
    nested_written = pydmg.extract_partition(dmg, 1, nested_partition_file)
    assert nested_written == len(partition_bytes)
    assert nested_partition_file.read_bytes() == partition_bytes


def test_partition_extract_replaces_link_without_following_it(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)
    outside = tmp_path / "outside.bin"
    outside.write_bytes(b"outside sentinel")
    destination = tmp_path / "partition-link.bin"
    _symlink_or_skip(destination, outside)

    expected = pydmg.read_partition(dmg, 1)
    written = pydmg.extract_partition(dmg, 1, destination)

    assert written == len(expected)
    assert not destination.is_symlink()
    assert destination.read_bytes() == expected
    assert outside.read_bytes() == b"outside sentinel"


def test_partition_extract_failure_preserves_existing_output(tmp_path: Path) -> None:
    source = Path(__file__).parent / "fixtures" / "fat32-large-sample.dmg"
    partition = pydmg.list_partitions(source)[1]
    chunk = next(
        item for item in partition["table"]["chunks"] if item["chunk_type"] == "Zlib"
    )
    contents = bytearray(source.read_bytes())
    contents[chunk["compressed_offset"]] ^= 0xFF
    malformed = tmp_path / "corrupt-zlib.dmg"
    malformed.write_bytes(contents)

    destination = tmp_path / "partition.bin"
    destination.write_bytes(b"existing output")
    with pytest.raises(RuntimeError):
        pydmg.extract_partition(malformed, 1, destination)

    assert destination.read_bytes() == b"existing output"


def test_fat32_listing_and_extraction(tmp_path: Path) -> None:
    source, dmg = _build_dmg(tmp_path)

    entries = pydmg.list_fat32_entries(dmg, partition_index=1)
    entry_paths = {item["path"] for item in entries}

    root_name = source.name
    assert f"{root_name}/hello.txt" in entry_paths
    assert f"{root_name}/docs/readme.md" in entry_paths

    out_dir = tmp_path / "extracted"
    extracted = pydmg.extract_fat32(dmg, out_dir, partition_index=1)
    assert f"{root_name}/hello.txt" in extracted

    assert (out_dir / root_name / "hello.txt").read_text(encoding="utf-8") == "hello from pydmg\n"
    assert (out_dir / root_name / "docs" / "readme.md").read_text(encoding="utf-8") == "sample file\n"

    with pytest.raises(RuntimeError):
        pydmg.extract_fat32(dmg, out_dir, partition_index=1, overwrite=False)

    extracted_overwrite = pydmg.extract_fat32(dmg, out_dir, partition_index=1, overwrite=True)
    assert f"{root_name}/hello.txt" in extracted_overwrite


def test_fat32_extraction_rejects_dangling_file_symlink(tmp_path: Path) -> None:
    source, dmg = _build_dmg(tmp_path)
    output_root = tmp_path / "extracted"
    image_directory = output_root / source.name
    image_directory.mkdir(parents=True)
    outside = tmp_path / "escaped.txt"
    _symlink_or_skip(image_directory / "hello.txt", outside)

    with pytest.raises(RuntimeError, match="symlink"):
        pydmg.extract_fat32(dmg, output_root, partition_index=1)

    assert not outside.exists()


def test_fat32_extraction_rejects_symlink_output_root(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)
    outside = tmp_path / "outside"
    outside.mkdir()
    output_root = tmp_path / "output-link"
    _symlink_or_skip(output_root, outside, target_is_directory=True)

    with pytest.raises(RuntimeError, match="symlink"):
        pydmg.extract_fat32(dmg, output_root, partition_index=1)

    assert list(outside.iterdir()) == []


def test_index_errors_are_exposed(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)

    with pytest.raises(IndexError):
        pydmg.read_partition(dmg, 99)

    with pytest.raises(IndexError):
        pydmg.list_fat32_entries(dmg, partition_index=99)

    with pytest.raises(IndexError):
        pydmg.inspect_gpt(dmg, partition_index=99)

    with pytest.raises(IndexError):
        pydmg.extract_fat32(dmg, tmp_path / "out", partition_index=99)


def test_checksum_helpers_match_inspect(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)

    checksum = pydmg.compute_data_checksum(dmg)
    valid = pydmg.verify_data_checksum(dmg)
    info = pydmg.inspect(dmg)

    assert checksum == info["checksum"]["computed_data_fork_crc32"]
    assert valid is True


def test_checksum_rejects_data_fork_range_beyond_eof(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)
    contents = bytearray(dmg.read_bytes())
    trailer = len(contents) - 512
    contents[trailer + 24 : trailer + 32] = (len(contents) + 1).to_bytes(8, "big")
    contents[trailer + 32 : trailer + 40] = (1).to_bytes(8, "big")
    contents[trailer + 88 : trailer + 92] = bytes(4)
    malformed = tmp_path / "out-of-range-checksum.dmg"
    malformed.write_bytes(contents)

    with pytest.raises(RuntimeError, match="data fork range"):
        pydmg.compute_data_checksum(malformed)
    with pytest.raises(RuntimeError, match="data fork range"):
        pydmg.verify_data_checksum(malformed)
    with pytest.raises(RuntimeError, match="data fork range"):
        pydmg.inspect(malformed)


def test_create_dmg_input_validation(tmp_path: Path) -> None:
    source = _build_source_tree(tmp_path)
    output = tmp_path / "invalid.dmg"

    with pytest.raises(ValueError):
        pydmg.create_dmg(source, output, total_sectors=0)

    with pytest.raises(ValueError):
        pydmg.create_dmg(tmp_path / "missing", output)

    file_source = tmp_path / "not_a_dir.txt"
    file_source.write_text("x", encoding="utf-8")
    with pytest.raises(ValueError):
        pydmg.create_dmg(file_source, output)

    with pytest.raises(ValueError, match="safety limit"):
        pydmg.create_dmg(source, output, total_sectors=1_048_577)


def test_create_dmg_replaces_link_without_following_it(tmp_path: Path) -> None:
    source = _build_source_tree(tmp_path)
    outside = tmp_path / "outside.bin"
    outside.write_bytes(b"outside sentinel")
    output = tmp_path / "output-link.dmg"
    _symlink_or_skip(output, outside)

    pydmg.create_dmg(source, output, total_sectors=32768)

    assert not output.is_symlink()
    assert pydmg.verify_data_checksum(output) is True
    assert outside.read_bytes() == b"outside sentinel"


def test_inspect_filesystems_reports_fat_metadata(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)

    info = pydmg.inspect_filesystems(dmg)

    assert isinstance(info["fat_filesystems"], list)
    assert info["apple_filesystem"] is None
    assert isinstance(info["errors"], list)
    assert len(info["fat_filesystems"]) >= 1

    fat = info["fat_filesystems"][0]
    assert fat["fat_type"] in {"fat12", "fat16", "fat32"}
    assert fat["cluster_size"] is not None
    assert fat["cluster_size"] > 0
    assert fat["total_clusters"] is not None
    assert fat["total_clusters"] > 0

    inspect_info = pydmg.inspect(dmg)
    assert inspect_info["fat_filesystems"] == info["fat_filesystems"]


def test_fat32_fixture_detected_as_fat32() -> None:
    fixture = Path(__file__).parent / "fixtures" / "fat32-large-sample.dmg"
    info = pydmg.inspect_filesystems(fixture)
    fat_types = {entry["fat_type"] for entry in info["fat_filesystems"]}
    assert "fat32" in fat_types


def test_hfs_fixture_listing_and_extract(tmp_path: Path) -> None:
    image = _extract_fixture_xz(tmp_path, "hfsp-small.img.xz")

    info = pydmg.inspect_filesystems(image)
    apple = info["apple_filesystem"]
    assert apple is not None
    assert apple["fs_type"] == "hfsplus"

    root_entries = pydmg.list_apple_entries(image, "/")
    assert len(root_entries) > 0

    root_entries_via_empty = pydmg.list_apple_entries(image, "")
    assert len(root_entries_via_empty) == len(root_entries)

    file_path = _find_first_regular_file(image)
    assert file_path is not None

    payload = pydmg.read_apple_file(image, file_path)
    assert isinstance(payload, (bytes, bytearray))

    output_file = tmp_path / "hfs_extracted.bin"
    written = pydmg.extract_apple_file(image, file_path, output_file)
    assert written == len(payload)
    assert output_file.read_bytes() == payload

    with pytest.raises(RuntimeError):
        pydmg.read_apple_file(image, "/definitely/does/not/exist")


def test_apfs_fixture_listing_and_extract(tmp_path: Path) -> None:
    image = Path(__file__).parent / "fixtures" / "apfs-linearmouse-v0.10.2.dmg"

    info = pydmg.inspect_filesystems(image)
    apple = info["apple_filesystem"]
    assert apple is not None
    assert apple["fs_type"] == "apfs"

    entries = pydmg.list_apple_entries(image, "/")
    assert len(entries) > 0

    file_path = _find_first_regular_file(image)
    assert file_path is not None

    payload = pydmg.read_apple_file(image, file_path)
    out_file = tmp_path / "apfs_extracted.bin"
    written = pydmg.extract_apple_file(image, file_path, out_file)
    assert written == len(payload)
    assert out_file.read_bytes() == payload


def test_bzip2_partition_decoder_on_apfs_fixture() -> None:
    image = Path(__file__).parent / "fixtures" / "apfs-linearmouse-v0.10.2.dmg"
    primary_gpt_header = pydmg.read_partition(image, 1)
    assert primary_gpt_header.startswith(b"EFI PART")


def test_apple_filesystem_helpers_raise_when_absent(tmp_path: Path) -> None:
    _, dmg = _build_dmg(tmp_path)

    with pytest.raises(RuntimeError):
        pydmg.list_apple_entries(dmg)

    with pytest.raises(RuntimeError):
        pydmg.read_apple_file(dmg, "/hello.txt")

    output = tmp_path / "hello.txt"
    output.write_bytes(b"original output")
    with pytest.raises(RuntimeError):
        pydmg.extract_apple_file(dmg, "/hello.txt", output)
    assert output.read_bytes() == b"original output"


def test_dmgimage_convenience_api(tmp_path: Path) -> None:
    source, dmg = _build_dmg(tmp_path)
    image = pydmg.DmgImage(dmg)

    assert image.checksum_valid() is True
    assert len(image.partitions()) >= 2
    assert f"{source.name}/hello.txt" in {
        entry["path"] for entry in image.list_fat32_entries(partition_index=1)
    }
    assert len(image.inspect_filesystems()["fat_filesystems"]) >= 1
    assert image.inspect_gpt()["has_gpt"] is False


def test_dmgimage_hfs_convenience_methods(tmp_path: Path) -> None:
    image_path = _extract_fixture_xz(tmp_path, "hfsp-small.img.xz")
    image = pydmg.DmgImage(image_path)

    fs_info = image.inspect_filesystems()
    assert fs_info["apple_filesystem"]["fs_type"] == "hfsplus"
    entries = image.list_apple_entries("/")
    assert len(entries) > 0

    file_path = _find_first_regular_file(image_path)
    assert file_path is not None
    blob = image.read_apple_file(file_path)
    out = tmp_path / "hfs_obj_extract.bin"
    written = image.extract_apple_file(file_path, out)
    assert written == len(blob)
    assert out.read_bytes() == blob
