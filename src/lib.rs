#![cfg_attr(all(feature = "fuzzing", not(feature = "python")), allow(dead_code))]

#[cfg(feature = "benchmarking")]
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicU64, Ordering},
};
use std::{
    any::Any,
    fs::{self, File},
    io::{self, BufReader, Cursor, Read, Seek, SeekFrom, Write},
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Component, Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use apple_dmg::{BlkxChunk, BlkxTable, ChunkType, KolyTrailer};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use bzip2::read::BzDecoder;
use crc32fast::Hasher;
use fatfs::{FatType, FileSystem, FsOptions, ReadWriteSeek};
use flate2::read::ZlibDecoder;
use gpt::{disk::LogicalBlockSize, GptConfig};
use plist::{Dictionary, Value};
#[cfg(feature = "python")]
use pyo3::{
    exceptions::{PyIndexError, PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyModule},
};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use tempfile::{tempfile, NamedTempFile};
use udif::{
    DmgArchive as UdifDmgArchive, DmgReaderOptions as UdifDmgReaderOptions,
    PartitionType as UdifPartitionType,
};

#[cfg(feature = "benchmarking")]
struct CountingAllocator;

#[cfg(feature = "benchmarking")]
static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmarking")]
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmarking")]
static REALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmarking")]
static REALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmarking")]
static DEALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmarking")]
static DEALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmarking")]
static STAGING_COPY_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmarking")]
static COMPRESSED_INPUT_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmarking")]
static DECODED_OUTPUT_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "benchmarking")]
static SINK_WRITTEN_BYTES: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "benchmarking")]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: this wrapper forwards the unchanged layout to the system allocator.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: this wrapper forwards the unchanged layout to the system allocator.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        DEALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
        DEALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: the caller supplies the pointer and layout accepted by GlobalAlloc.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: this wrapper forwards the allocation and requested size unchanged.
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() {
            REALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            REALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        new_pointer
    }
}

#[cfg(feature = "benchmarking")]
#[global_allocator]
static BENCHMARK_ALLOCATOR: CountingAllocator = CountingAllocator;

#[cfg(feature = "benchmarking")]
#[derive(Serialize)]
struct BenchmarkMetrics {
    allocation_calls: u64,
    allocated_bytes: u64,
    reallocation_calls: u64,
    reallocated_bytes: u64,
    deallocation_calls: u64,
    deallocated_bytes: u64,
    staging_copy_bytes: u64,
    compressed_input_bytes: u64,
    decoded_output_bytes: u64,
    sink_written_bytes: u64,
}

#[cfg(feature = "benchmarking")]
fn reset_benchmark_metrics() {
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    REALLOCATION_CALLS.store(0, Ordering::Relaxed);
    REALLOCATED_BYTES.store(0, Ordering::Relaxed);
    DEALLOCATION_CALLS.store(0, Ordering::Relaxed);
    DEALLOCATED_BYTES.store(0, Ordering::Relaxed);
    STAGING_COPY_BYTES.store(0, Ordering::Relaxed);
    COMPRESSED_INPUT_BYTES.store(0, Ordering::Relaxed);
    DECODED_OUTPUT_BYTES.store(0, Ordering::Relaxed);
    SINK_WRITTEN_BYTES.store(0, Ordering::Relaxed);
}

#[cfg(feature = "benchmarking")]
fn benchmark_metrics() -> BenchmarkMetrics {
    BenchmarkMetrics {
        allocation_calls: ALLOCATION_CALLS.load(Ordering::Relaxed),
        allocated_bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
        reallocation_calls: REALLOCATION_CALLS.load(Ordering::Relaxed),
        reallocated_bytes: REALLOCATED_BYTES.load(Ordering::Relaxed),
        deallocation_calls: DEALLOCATION_CALLS.load(Ordering::Relaxed),
        deallocated_bytes: DEALLOCATED_BYTES.load(Ordering::Relaxed),
        staging_copy_bytes: STAGING_COPY_BYTES.load(Ordering::Relaxed),
        compressed_input_bytes: COMPRESSED_INPUT_BYTES.load(Ordering::Relaxed),
        decoded_output_bytes: DECODED_OUTPUT_BYTES.load(Ordering::Relaxed),
        sink_written_bytes: SINK_WRITTEN_BYTES.load(Ordering::Relaxed),
    }
}

#[cfg(feature = "benchmarking")]
fn record_staging_copy(bytes: usize) {
    STAGING_COPY_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

#[cfg(not(feature = "benchmarking"))]
fn record_staging_copy(_bytes: usize) {}

#[cfg(feature = "benchmarking")]
fn record_decode(input_bytes: u64, output_bytes: usize) {
    COMPRESSED_INPUT_BYTES.fetch_add(input_bytes, Ordering::Relaxed);
    DECODED_OUTPUT_BYTES.fetch_add(output_bytes as u64, Ordering::Relaxed);
}

#[cfg(not(feature = "benchmarking"))]
fn record_decode(_input_bytes: u64, _output_bytes: usize) {}

#[cfg(feature = "benchmarking")]
fn record_sink_write(bytes: u64) {
    SINK_WRITTEN_BYTES.fetch_add(bytes, Ordering::Relaxed);
}

#[cfg(not(feature = "benchmarking"))]
fn record_sink_write(_bytes: u64) {}

const SECTOR_SIZE: u64 = 512;
const KOLY_TRAILER_SIZE: u64 = 512;
const BLKX_HEADER_SIZE: usize = 204;
const BLKX_CHUNK_SIZE: usize = 40;
const BLKX_CHUNK_COUNT_OFFSET: usize = 200;
const MAX_PLIST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PLIST_DEPTH: usize = 128;
const MAX_PLIST_NODES: usize = 1_000_000;
const MAX_BLKX_CHUNKS: usize = 262_144;
const MAX_PARTITION_BYTES: u64 = 512 * 1024 * 1024;
const MAX_COMPRESSED_CHUNK_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED_CHUNK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_APPLE_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILESYSTEM_ENTRIES: usize = 100_000;
const MAX_FILESYSTEM_DEPTH: usize = 128;
const MAX_METADATA_CANDIDATES: usize = 10_000;
const MAX_METADATA_VALUE_CHARS: usize = 4_096;
const MAX_GPT_PARTITIONS: u32 = 16_384;
const MAX_FILESYSTEM_DEPENDENCY_READ_BYTES: u64 = MAX_PARTITION_BYTES + 64 * 1024 * 1024;
const MAX_FILESYSTEM_DEPENDENCY_READ_OPERATIONS: u64 = 2_000_000;
const HFS_VOLUME_HEADER_OFFSET: u64 = 1_024;
const HFS_VOLUME_HEADER_PREFIX_SIZE: usize = 48;
const HFS_MIN_ALLOCATION_BLOCK_BYTES: u32 = 512;
const MAX_HFS_ALLOCATION_BLOCK_BYTES: u32 = 64 * 1024 * 1024;
const APFS_SUPERBLOCK_PREFIX_SIZE: usize = 48;
const APFS_MIN_BLOCK_BYTES: u32 = 4_096;
const APFS_MAX_BLOCK_BYTES: u32 = 65_536;
const MAX_APFS_CHECKPOINT_BLOCKS: u32 = 4_096;

#[derive(Debug)]
struct ReadOnlyDevice<'a> {
    inner: Cursor<&'a [u8]>,
}

impl<'a> ReadOnlyDevice<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            inner: Cursor::new(bytes),
        }
    }
}

impl Read for ReadOnlyDevice<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Write for ReadOnlyDevice<'_> {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "read-only parser device",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for ReadOnlyDevice<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

#[derive(Debug)]
struct BudgetedIo<T> {
    inner: T,
    read_bytes: u64,
    read_operations: u64,
    max_read_bytes: u64,
    max_read_operations: u64,
}

impl<T> BudgetedIo<T> {
    fn new(inner: T) -> Self {
        Self::with_limits(
            inner,
            MAX_FILESYSTEM_DEPENDENCY_READ_BYTES,
            MAX_FILESYSTEM_DEPENDENCY_READ_OPERATIONS,
        )
    }

    fn with_limits(inner: T, max_read_bytes: u64, max_read_operations: u64) -> Self {
        Self {
            inner,
            read_bytes: 0,
            read_operations: 0,
            max_read_bytes,
            max_read_operations,
        }
    }
}

impl<T: Read> Read for BudgetedIo<T> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return self.inner.read(buffer);
        }

        self.read_operations = self.read_operations.checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "filesystem dependency read operation count overflow",
            )
        })?;
        if self.read_operations > self.max_read_operations {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "filesystem dependency read operation budget exceeded",
            ));
        }

        let remaining = self
            .max_read_bytes
            .checked_sub(self.read_bytes)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "filesystem dependency read byte count overflow",
                )
            })?;
        if remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "filesystem dependency read byte budget exceeded",
            ));
        }

        let allowed = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let read = self.inner.read(&mut buffer[..allowed])?;
        self.read_bytes = self.read_bytes.checked_add(read as u64).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "filesystem dependency read byte count overflow",
            )
        })?;
        Ok(read)
    }
}

impl<T: Write> Write for BudgetedIo<T> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<T: Seek> Seek for BudgetedIo<T> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

#[derive(Debug)]
struct ParsedDmg {
    file_size: u64,
    koly: KolyTrailer,
    plist: Value,
    partitions: Vec<PartitionRecord>,
}

#[derive(Debug)]
struct ParsedGpt {
    logical_block_size: u64,
    disk_guid: String,
    primary_header: Option<GptHeaderInfo>,
    backup_header: Option<GptHeaderInfo>,
    validation: GptValidation,
    partitions: Vec<GptPartitionInfo>,
}

#[derive(Debug)]
struct PartitionRecord {
    index: usize,
    id: String,
    name: String,
    cfname: Option<String>,
    attributes: Option<String>,
    table: BlkxTable,
}

#[derive(Debug, Serialize)]
struct DmgInfo {
    path: String,
    file_size: u64,
    koly: KolyInfo,
    checksum: ChecksumInfo,
    plist: JsonValue,
    metadata_candidates: MetadataCandidates,
    partitions: Vec<PartitionInfo>,
    fat_filesystems: Vec<FatFilesystemMetadata>,
    apple_filesystem: Option<AppleFilesystemMetadata>,
    filesystem_errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ChecksumInfo {
    declared_data_fork_crc32: u32,
    computed_data_fork_crc32: u32,
    data_fork_checksum_matches: bool,
    declared_main_crc32: u32,
}

#[derive(Debug, Serialize)]
struct KolyInfo {
    version: u32,
    flags: u32,
    running_data_fork_offset: u64,
    data_fork_offset: u64,
    data_fork_length: u64,
    resource_fork_offset: u64,
    resource_fork_length: u64,
    segment_number: u32,
    segment_count: u32,
    segment_id_hex: String,
    plist_offset: u64,
    plist_length: u64,
    code_signature_offset: u64,
    code_signature_size: u64,
    image_variant: u32,
    sector_count: u64,
    data_fork_digest_crc32: u32,
    main_digest_crc32: u32,
}

#[derive(Debug, Serialize)]
struct PartitionInfo {
    index: usize,
    id: String,
    name: String,
    cfname: Option<String>,
    attributes: Option<String>,
    table: BlkxTableInfo,
}

#[derive(Debug, Serialize)]
struct BlkxTableInfo {
    version: u32,
    sector_number: u64,
    sector_count: u64,
    data_offset: u64,
    buffers_needed: u32,
    block_descriptors: u32,
    checksum_crc32: u32,
    chunk_count: usize,
    chunks: Vec<BlkxChunkInfo>,
}

#[derive(Debug, Serialize)]
struct BlkxChunkInfo {
    chunk_type: Option<String>,
    raw_type: u32,
    comment: u32,
    sector_number: u64,
    sector_count: u64,
    compressed_offset: u64,
    compressed_length: u64,
}

#[derive(Debug, Serialize, Default)]
struct MetadataCandidates {
    creation_dates: Vec<MetadataCandidate>,
    authors: Vec<MetadataCandidate>,
    creation_applications: Vec<MetadataCandidate>,
}

#[derive(Debug, Serialize)]
struct MetadataCandidate {
    path: String,
    key: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct Fat32Entry {
    path: String,
    is_dir: bool,
    size: Option<u64>,
}

#[derive(Debug, Serialize)]
struct FatFilesystemMetadata {
    partition_index: usize,
    partition_name: String,
    fat_type: String,
    volume_id: u32,
    volume_label: String,
    root_volume_label: Option<String>,
    cluster_size: Option<u32>,
    total_clusters: Option<u32>,
    free_clusters: Option<u32>,
}

#[derive(Debug, Serialize)]
struct AppleFilesystemMetadata {
    fs_type: String,
    block_size: u32,
    file_count: u64,
    directory_count: u64,
    volume_name: Option<String>,
    symlink_count: Option<u64>,
    total_blocks: Option<u32>,
    free_blocks: Option<u32>,
    version: Option<u16>,
    is_hfsx: Option<bool>,
    volume_create_time: Option<i64>,
    volume_modify_time: Option<i64>,
    root_create_time: Option<i64>,
    root_modify_time: Option<i64>,
}

#[derive(Debug, Serialize)]
struct AppleFsEntry {
    path: String,
    is_dir: bool,
    size: u64,
}

#[derive(Debug, Serialize)]
struct FilesystemsInspection {
    path: String,
    fat_filesystems: Vec<FatFilesystemMetadata>,
    apple_filesystem: Option<AppleFilesystemMetadata>,
    errors: Vec<String>,
}

enum AppleFsHandle {
    Hfs(Box<hfsplus::HfsVolume<BudgetedIo<BufReader<File>>>>),
    Apfs(Box<apfs::ApfsVolume<BudgetedIo<BufReader<File>>>>),
}

#[derive(Debug, Serialize)]
struct GptInspection {
    path: String,
    partition_index: Option<usize>,
    has_gpt: bool,
    logical_block_size: Option<u64>,
    disk_guid: Option<String>,
    primary_header: Option<GptHeaderInfo>,
    backup_header: Option<GptHeaderInfo>,
    validation: GptValidation,
    partitions: Vec<GptPartitionInfo>,
    errors: Vec<String>,
}

#[derive(Debug, Serialize, Default)]
struct GptValidation {
    primary_header_valid: bool,
    backup_header_valid: bool,
    using_backup_as_fallback: bool,
}

#[derive(Debug, Serialize)]
struct GptHeaderInfo {
    signature: String,
    revision_major: u16,
    revision_minor: u16,
    header_size: u32,
    crc32: u32,
    current_lba: u64,
    backup_lba: u64,
    first_usable_lba: u64,
    last_usable_lba: u64,
    disk_guid: String,
    partition_table_start_lba: u64,
    partition_count: u32,
    partition_entry_size: u32,
    partition_table_crc32: u32,
}

#[derive(Debug, Serialize)]
struct GptPartitionInfo {
    entry_index: u32,
    type_guid: String,
    type_os: String,
    unique_guid: String,
    name: String,
    first_lba: u64,
    last_lba: u64,
    sector_count: u64,
    byte_size: u64,
    flags: u64,
}

fn parse_dmg(path: &Path) -> Result<ParsedDmg> {
    let file =
        File::open(path).with_context(|| format!("unable to open DMG: {}", path.display()))?;
    let file_size = file
        .metadata()
        .with_context(|| format!("unable to stat DMG: {}", path.display()))?
        .len();
    let mut reader = BufReader::new(file);

    parse_dmg_reader(&mut reader, file_size)
}

fn validate_file_range(file_size: u64, offset: u64, length: u64, label: &str) -> Result<()> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| anyhow!("{label} range overflows u64"))?;
    if offset > file_size || end > file_size {
        bail!("{label} range {offset}..{end} exceeds file size {file_size}");
    }
    Ok(())
}

fn validate_koly_ranges(koly: &KolyTrailer, file_size: u64) -> Result<()> {
    if file_size < KOLY_TRAILER_SIZE {
        bail!("DMG is smaller than the {KOLY_TRAILER_SIZE}-byte koly trailer");
    }

    validate_file_range(
        file_size,
        koly.data_fork_offset,
        koly.data_fork_length,
        "data fork",
    )?;
    validate_file_range(file_size, koly.plist_offset, koly.plist_length, "plist")?;

    if koly.resource_fork_length != 0 {
        validate_file_range(
            file_size,
            koly.resource_fork_offset,
            koly.resource_fork_length,
            "resource fork",
        )?;
    }
    if koly.code_signature_size != 0 {
        validate_file_range(
            file_size,
            koly.code_signature_offset,
            koly.code_signature_size,
            "code signature",
        )?;
    }
    if koly.plist_length > MAX_PLIST_BYTES {
        bail!(
            "plist length {} exceeds safety limit {}",
            koly.plist_length,
            MAX_PLIST_BYTES
        );
    }
    Ok(())
}

fn validate_plist_structure(root: &Value) -> Result<()> {
    let mut stack = vec![(root, 0_usize)];
    let mut nodes = 0_usize;

    while let Some((value, depth)) = stack.pop() {
        nodes = nodes
            .checked_add(1)
            .ok_or_else(|| anyhow!("plist node count overflow"))?;
        if nodes > MAX_PLIST_NODES {
            bail!("plist exceeds node limit {MAX_PLIST_NODES}");
        }
        if depth > MAX_PLIST_DEPTH {
            bail!("plist exceeds nesting-depth limit {MAX_PLIST_DEPTH}");
        }

        match value {
            Value::Array(values) => {
                for child in values {
                    stack.push((child, depth + 1));
                }
            }
            Value::Dictionary(dict) => {
                for child in dict.values() {
                    stack.push((child, depth + 1));
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn read_binary_plist_uint(bytes: &[u8], offset: usize, size: usize, label: &str) -> Result<u64> {
    if size == 0 || size > 8 {
        bail!("invalid binary plist {label} integer size {size}");
    }
    let end = offset
        .checked_add(size)
        .ok_or_else(|| anyhow!("binary plist {label} range overflow"))?;
    let field = bytes
        .get(offset..end)
        .ok_or_else(|| anyhow!("binary plist {label} is truncated"))?;
    Ok(field
        .iter()
        .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte)))
}

fn binary_plist_collection_length(
    bytes: &[u8],
    marker_offset: usize,
    short_length: u8,
    object_table_end: usize,
) -> Result<(u64, usize)> {
    if short_length < 0x0f {
        return Ok((u64::from(short_length), marker_offset + 1));
    }

    let length_marker_offset = marker_offset
        .checked_add(1)
        .ok_or_else(|| anyhow!("binary plist collection length offset overflow"))?;
    let length_marker = *bytes
        .get(length_marker_offset)
        .ok_or_else(|| anyhow!("binary plist collection length marker is truncated"))?;
    if length_marker >> 4 != 0x01 || length_marker & 0x0f > 3 {
        bail!("binary plist collection has an invalid extended length marker");
    }
    let length_size = 1_usize << usize::from(length_marker & 0x0f);
    let length_offset = length_marker_offset
        .checked_add(1)
        .ok_or_else(|| anyhow!("binary plist collection length offset overflow"))?;
    let length = read_binary_plist_uint(bytes, length_offset, length_size, "collection length")?;
    let references_offset = length_offset
        .checked_add(length_size)
        .ok_or_else(|| anyhow!("binary plist collection reference offset overflow"))?;
    if references_offset > object_table_end {
        bail!("binary plist collection length extends beyond the object table");
    }
    Ok((length, references_offset))
}

fn preflight_binary_plist(bytes: &[u8]) -> Result<()> {
    if !bytes.starts_with(b"bplist00") {
        return Ok(());
    }
    if bytes.len() < 40 {
        bail!("binary plist is too short for its header and trailer");
    }

    let trailer_offset = bytes.len() - 32;
    let trailer = &bytes[trailer_offset..];
    let offset_size = usize::from(trailer[6]);
    let reference_size = usize::from(trailer[7]);
    if !matches!(offset_size, 1 | 2 | 3 | 4 | 8) {
        bail!("invalid binary plist object-offset size {offset_size}");
    }
    if !matches!(reference_size, 1 | 2 | 3 | 4 | 8) {
        bail!("invalid binary plist object-reference size {reference_size}");
    }

    let object_count = read_binary_plist_uint(trailer, 8, 8, "object count")?;
    if object_count == 0 {
        bail!("binary plist declares no objects");
    }
    if object_count > MAX_PLIST_NODES as u64 {
        bail!("binary plist object count {object_count} exceeds node limit {MAX_PLIST_NODES}");
    }
    let root_object = read_binary_plist_uint(trailer, 16, 8, "root object")?;
    if root_object >= object_count {
        bail!("binary plist root object {root_object} is out of range for {object_count} objects");
    }

    let offset_table_offset_u64 = read_binary_plist_uint(trailer, 24, 8, "offset-table offset")?;
    let offset_table_offset = usize::try_from(offset_table_offset_u64)
        .context("binary plist offset-table offset exceeds usize")?;
    let object_count_usize =
        usize::try_from(object_count).context("binary plist object count exceeds usize")?;
    let offset_table_length = object_count_usize
        .checked_mul(offset_size)
        .ok_or_else(|| anyhow!("binary plist offset-table length overflow"))?;
    let offset_table_end = offset_table_offset
        .checked_add(offset_table_length)
        .ok_or_else(|| anyhow!("binary plist offset-table range overflow"))?;
    if offset_table_offset < 8 || offset_table_end > trailer_offset {
        bail!("binary plist offset table is outside the object-table region");
    }

    for index in 0..object_count_usize {
        let field_offset = offset_table_offset
            .checked_add(
                index
                    .checked_mul(offset_size)
                    .ok_or_else(|| anyhow!("binary plist object-offset index overflow"))?,
            )
            .ok_or_else(|| anyhow!("binary plist object-offset field overflow"))?;
        let object_offset_u64 =
            read_binary_plist_uint(bytes, field_offset, offset_size, "object offset")?;
        let object_offset = usize::try_from(object_offset_u64)
            .context("binary plist object offset exceeds usize")?;
        let marker = *bytes
            .get(object_offset)
            .filter(|_| object_offset >= 8 && object_offset < offset_table_offset)
            .ok_or_else(|| anyhow!("binary plist object {index} offset is out of bounds"))?;
        let object_type = marker >> 4;
        if !matches!(object_type, 0x0a | 0x0d) {
            continue;
        }

        let (collection_length, references_offset) = binary_plist_collection_length(
            bytes,
            object_offset,
            marker & 0x0f,
            offset_table_offset,
        )?;
        if collection_length > MAX_PLIST_NODES as u64 {
            bail!(
                "binary plist collection length {collection_length} exceeds node limit {MAX_PLIST_NODES}"
            );
        }
        let reference_count = if object_type == 0x0d {
            collection_length
                .checked_mul(2)
                .ok_or_else(|| anyhow!("binary plist dictionary reference count overflow"))?
        } else {
            collection_length
        };
        let reference_bytes = reference_count
            .checked_mul(reference_size as u64)
            .ok_or_else(|| anyhow!("binary plist collection reference length overflow"))?;
        let references_end = u64::try_from(references_offset)
            .context("binary plist reference offset exceeds u64")?
            .checked_add(reference_bytes)
            .ok_or_else(|| anyhow!("binary plist collection reference range overflow"))?;
        if references_end > offset_table_offset_u64 {
            bail!("binary plist collection references extend beyond the object table");
        }
    }

    Ok(())
}

struct BoundedPlistEvents<I> {
    inner: I,
    nodes: usize,
    depth: usize,
    stopped: bool,
    limit_error: Option<String>,
}

impl<I> BoundedPlistEvents<I> {
    fn new(inner: I) -> Self {
        Self {
            inner,
            nodes: 0,
            depth: 0,
            stopped: false,
            limit_error: None,
        }
    }

    fn stop(&mut self, message: String) {
        self.stopped = true;
        self.limit_error = Some(message);
    }
}

impl<I> Iterator for BoundedPlistEvents<I>
where
    I: Iterator<Item = std::result::Result<plist::stream::OwnedEvent, plist::Error>>,
{
    type Item = std::result::Result<plist::stream::OwnedEvent, plist::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.stopped {
            return None;
        }
        let event = self.inner.next()?;
        let Ok(event) = event else {
            return Some(event);
        };

        match &event {
            plist::stream::Event::EndCollection => {
                self.depth = self.depth.saturating_sub(1);
                return Some(Ok(event));
            }
            plist::stream::Event::StartArray(length)
            | plist::stream::Event::StartDictionary(length) => {
                if length.is_some_and(|length| length > MAX_PLIST_NODES as u64) {
                    self.stop(format!(
                        "plist collection length exceeds node limit {MAX_PLIST_NODES}"
                    ));
                    return None;
                }
                if self.depth > MAX_PLIST_DEPTH {
                    self.stop(format!(
                        "plist exceeds nesting-depth limit {MAX_PLIST_DEPTH}"
                    ));
                    return None;
                }
                self.depth = self.depth.saturating_add(1);
            }
            _ => {
                if self.depth > MAX_PLIST_DEPTH {
                    self.stop(format!(
                        "plist exceeds nesting-depth limit {MAX_PLIST_DEPTH}"
                    ));
                    return None;
                }
            }
        }

        self.nodes = match self.nodes.checked_add(1) {
            Some(nodes) => nodes,
            None => {
                self.stop("plist node count overflow".to_string());
                return None;
            }
        };
        if self.nodes > MAX_PLIST_NODES {
            self.stop(format!("plist exceeds node limit {MAX_PLIST_NODES}"));
            return None;
        }
        Some(Ok(event))
    }
}

fn parse_plist_payload(bytes: &[u8]) -> Result<Value> {
    preflight_binary_plist(bytes)?;
    let reader = plist::stream::Reader::new(Cursor::new(bytes));
    let mut events = BoundedPlistEvents::new(reader);
    let parsed = Value::from_events(&mut events);
    if let Some(error) = events.limit_error {
        bail!(error);
    }
    let plist = parsed.context("unable to parse plist payload")?;
    validate_plist_structure(&plist)?;
    Ok(plist)
}

fn parse_dmg_reader<R: Read + Seek>(reader: &mut R, file_size: u64) -> Result<ParsedDmg> {
    let koly = KolyTrailer::read_from(reader).context("unable to read koly trailer")?;
    validate_koly_ranges(&koly, file_size)?;

    reader
        .seek(SeekFrom::Start(koly.plist_offset))
        .context("unable to seek to plist")?;
    let plist_len =
        usize::try_from(koly.plist_length).context("plist length does not fit usize")?;
    let mut plist_bytes = vec![0_u8; plist_len];
    reader
        .read_exact(&mut plist_bytes)
        .context("unable to read plist payload")?;

    let plist = parse_plist_payload(&plist_bytes)?;

    let partitions = extract_partitions(&plist)?;

    Ok(ParsedDmg {
        file_size,
        koly,
        plist,
        partitions,
    })
}

fn decode_blkx_table(bytes: &[u8]) -> Result<BlkxTable> {
    if bytes.len() < BLKX_HEADER_SIZE {
        bail!(
            "blkx table is truncated: {} bytes, expected at least {BLKX_HEADER_SIZE}",
            bytes.len()
        );
    }
    if &bytes[..4] != b"mish" {
        bail!("blkx table has invalid signature");
    }

    let count_bytes: [u8; 4] = bytes[BLKX_CHUNK_COUNT_OFFSET..BLKX_HEADER_SIZE]
        .try_into()
        .expect("fixed-size blkx chunk-count field");
    let chunk_count = usize::try_from(u32::from_be_bytes(count_bytes))
        .context("blkx chunk count does not fit usize")?;
    if chunk_count > MAX_BLKX_CHUNKS {
        bail!("blkx chunk count {chunk_count} exceeds limit {MAX_BLKX_CHUNKS}");
    }

    let chunk_bytes = chunk_count
        .checked_mul(BLKX_CHUNK_SIZE)
        .ok_or_else(|| anyhow!("blkx chunk table size overflow"))?;
    let required = BLKX_HEADER_SIZE
        .checked_add(chunk_bytes)
        .ok_or_else(|| anyhow!("blkx table size overflow"))?;
    if required > bytes.len() {
        bail!(
            "blkx declares {chunk_count} chunks requiring {required} bytes, but table has {}",
            bytes.len()
        );
    }

    let table =
        BlkxTable::read_from(&mut Cursor::new(bytes)).context("unable to parse blkx table")?;
    let expanded_size = table
        .sector_count
        .checked_mul(SECTOR_SIZE)
        .ok_or_else(|| anyhow!("blkx expanded size overflow"))?;
    if expanded_size > MAX_PARTITION_BYTES {
        bail!("blkx expanded size {expanded_size} exceeds limit {MAX_PARTITION_BYTES}");
    }

    let mut expected_sector = 0_u64;
    let mut total_compressed = 0_u64;
    let mut saw_terminator = false;

    for (index, chunk) in table.chunks.iter().enumerate() {
        let sector_end = chunk
            .sector_number
            .checked_add(chunk.sector_count)
            .ok_or_else(|| anyhow!("blkx chunk {index} sector range overflow"))?;
        chunk
            .compressed_offset
            .checked_add(chunk.compressed_length)
            .ok_or_else(|| anyhow!("blkx chunk {index} compressed range overflow"))?;
        if chunk.compressed_length > MAX_COMPRESSED_CHUNK_BYTES {
            bail!(
                "blkx chunk {index} compressed length {} exceeds limit {}",
                chunk.compressed_length,
                MAX_COMPRESSED_CHUNK_BYTES
            );
        }

        total_compressed = total_compressed
            .checked_add(chunk.compressed_length)
            .ok_or_else(|| anyhow!("blkx aggregate compressed length overflow"))?;
        if total_compressed > MAX_PARTITION_BYTES {
            bail!(
                "blkx aggregate compressed length {total_compressed} exceeds limit {MAX_PARTITION_BYTES}"
            );
        }

        match chunk.ty() {
            Some(ChunkType::Comment) => continue,
            Some(ChunkType::Term) => {
                if saw_terminator {
                    bail!("blkx contains multiple terminator chunks");
                }
                if chunk.sector_count != 0 || chunk.compressed_length != 0 {
                    bail!("blkx terminator chunk {index} contains data");
                }
                if chunk.sector_number != expected_sector {
                    bail!(
                        "blkx terminator starts at sector {}, expected {expected_sector}",
                        chunk.sector_number
                    );
                }
                saw_terminator = true;
                continue;
            }
            _ => {}
        }

        if saw_terminator {
            bail!("blkx data chunk {index} appears after the terminator");
        }
        if chunk.sector_count == 0 {
            bail!("blkx data chunk {index} has zero sectors");
        }
        if chunk.sector_number != expected_sector {
            bail!(
                "blkx data chunk {index} starts at sector {}, expected {expected_sector}",
                chunk.sector_number
            );
        }
        if sector_end > table.sector_count {
            bail!(
                "blkx data chunk {index} ends at sector {sector_end}, beyond partition sector count {}",
                table.sector_count
            );
        }

        let chunk_expanded = chunk
            .sector_count
            .checked_mul(SECTOR_SIZE)
            .ok_or_else(|| anyhow!("blkx chunk {index} expanded size overflow"))?;
        if chunk_expanded > MAX_EXPANDED_CHUNK_BYTES {
            bail!(
                "blkx chunk {index} expanded size {chunk_expanded} exceeds limit {MAX_EXPANDED_CHUNK_BYTES}"
            );
        }
        if matches!(chunk.ty(), Some(ChunkType::Raw | ChunkType::Ignore))
            && chunk.compressed_length > chunk_expanded
        {
            bail!(
                "blkx raw chunk {index} compressed length {} exceeds expanded size {chunk_expanded}",
                chunk.compressed_length
            );
        }
        expected_sector = sector_end;
    }

    if !saw_terminator {
        bail!("blkx table is missing a terminator chunk");
    }
    if expected_sector != table.sector_count {
        bail!(
            "blkx chunks cover {expected_sector} sectors, expected {}",
            table.sector_count
        );
    }

    Ok(table)
}

fn extract_partitions(plist: &Value) -> Result<Vec<PartitionRecord>> {
    let root = expect_dict(plist, "plist root")?;
    let resource_fork_value = root
        .get("resource-fork")
        .or_else(|| root.get("resourceFork"))
        .context("plist missing resource-fork")?;
    let resource_fork = expect_dict(resource_fork_value, "resource-fork")?;
    let blkx_value = resource_fork
        .get("blkx")
        .context("resource-fork missing blkx")?;
    let blkx_entries = expect_array(blkx_value, "resource-fork.blkx")?;

    if blkx_entries.len() > MAX_BLKX_CHUNKS {
        bail!(
            "resource-fork.blkx entry count {} exceeds limit {MAX_BLKX_CHUNKS}",
            blkx_entries.len()
        );
    }

    let mut partitions = Vec::with_capacity(blkx_entries.len());

    for (source_index, entry) in blkx_entries.iter().enumerate() {
        let dict = expect_dict(entry, "partition entry")?;
        let Some(data_value) = dict.get("Data") else {
            continue;
        };
        let table_bytes = match data_value {
            Value::Data(bytes) => bytes,
            _ => continue,
        };

        let table = decode_blkx_table(table_bytes)
            .with_context(|| format!("unable to decode blkx table for partition {source_index}"))?;
        let logical_index = partitions.len();

        let id = dict
            .get("ID")
            .map(value_to_short_string)
            .unwrap_or_else(|| source_index.to_string());
        let name = dict
            .get("Name")
            .map(value_to_short_string)
            .unwrap_or_else(|| format!("partition-{source_index}"));
        let cfname = dict.get("CFName").map(value_to_short_string);
        let attributes = dict.get("Attributes").map(value_to_short_string);

        partitions.push(PartitionRecord {
            index: logical_index,
            id,
            name,
            cfname,
            attributes,
            table,
        });
    }

    if partitions.is_empty() {
        bail!("no readable partition entries found in plist resource-fork.blkx");
    }

    Ok(partitions)
}

fn expect_dict<'a>(value: &'a Value, name: &str) -> Result<&'a Dictionary> {
    match value {
        Value::Dictionary(dict) => Ok(dict),
        _ => bail!("{name} is not a dictionary"),
    }
}

fn expect_array<'a>(value: &'a Value, name: &str) -> Result<&'a Vec<Value>> {
    match value {
        Value::Array(values) => Ok(values),
        _ => bail!("{name} is not an array"),
    }
}

#[derive(Default)]
struct DecodeBuffers {
    compressed: Vec<u8>,
    decoded: Vec<u8>,
}

fn seek_to_payload<R: Read + Seek>(
    reader: &mut R,
    input_size: u64,
    offset: u64,
    length: u64,
) -> Result<()> {
    validate_file_range(input_size, offset, length, "chunk payload")?;
    if length > MAX_COMPRESSED_CHUNK_BYTES {
        bail!("chunk payload length {length} exceeds limit {MAX_COMPRESSED_CHUNK_BYTES}");
    }
    reader
        .seek(SeekFrom::Start(offset))
        .with_context(|| format!("unable to seek to payload at offset {offset}"))?;
    Ok(())
}

fn read_exact_at_into<R: Read + Seek>(
    reader: &mut R,
    input_size: u64,
    offset: u64,
    length: u64,
    buffer: &mut Vec<u8>,
) -> Result<()> {
    seek_to_payload(reader, input_size, offset, length)?;
    let len = usize::try_from(length).context("payload length does not fit usize")?;
    buffer.clear();
    buffer
        .try_reserve(len)
        .context("unable to reserve compressed-chunk buffer")?;
    buffer.resize(len, 0);
    reader
        .read_exact(buffer)
        .with_context(|| format!("unable to read payload at offset {offset}, length {length}"))?;
    Ok(())
}

fn bounded_copy<R: Read, W: Write>(
    decoder: R,
    output_limit: u64,
    label: &str,
    writer: &mut W,
) -> Result<u64> {
    let read_limit = output_limit
        .checked_add(1)
        .ok_or_else(|| anyhow!("{label} output limit overflow"))?;
    let mut limited = decoder.take(read_limit);
    let written = io::copy(&mut limited, writer)
        .with_context(|| format!("unable to {label}-decompress chunk"))?;
    if written > output_limit {
        bail!("{label} output exceeds limit {output_limit}");
    }
    Ok(written)
}

fn compressed_output_limit(chunk: &BlkxChunk, remaining_output: u64) -> Result<u64> {
    let declared = if chunk.sector_count == 0 {
        remaining_output
    } else {
        chunk
            .sector_count
            .checked_mul(SECTOR_SIZE)
            .ok_or_else(|| anyhow!("chunk expanded size overflow"))?
    };
    if declared > remaining_output {
        bail!(
            "chunk expanded size {declared} exceeds remaining partition limit {remaining_output}"
        );
    }
    Ok(declared)
}

fn write_zeroes<W: Write>(writer: &mut W, length: u64) -> Result<()> {
    static ZEROES: [u8; 128 * 1024] = [0; 128 * 1024];
    let mut remaining = length;
    while remaining != 0 {
        let count = usize::try_from(remaining.min(ZEROES.len() as u64))
            .context("zero-fill write length does not fit usize")?;
        writer
            .write_all(&ZEROES[..count])
            .context("unable to write zero-fill chunk")?;
        remaining -= count as u64;
    }
    Ok(())
}

fn decode_chunk_to<R: Read + Seek, W: Write>(
    reader: &mut R,
    input_size: u64,
    chunk: &BlkxChunk,
    remaining_output: u64,
    writer: &mut W,
    buffers: &mut DecodeBuffers,
) -> Result<u64> {
    let chunk_type = chunk
        .ty()
        .ok_or_else(|| anyhow!("unknown chunk type: 0x{:08x}", chunk.r#type))?;

    let written = match chunk_type {
        ChunkType::Raw => {
            if chunk.compressed_length > remaining_output {
                bail!(
                    "raw chunk length {} exceeds remaining partition limit {remaining_output}",
                    chunk.compressed_length
                );
            }
            seek_to_payload(
                reader,
                input_size,
                chunk.compressed_offset,
                chunk.compressed_length,
            )?;
            let mut payload = reader.take(chunk.compressed_length);
            let written = io::copy(&mut payload, writer).context("unable to copy raw chunk")?;
            if written != chunk.compressed_length {
                bail!(
                    "raw chunk ended after {written} bytes; expected {}",
                    chunk.compressed_length
                );
            }
            written
        }
        ChunkType::Zlib => {
            seek_to_payload(
                reader,
                input_size,
                chunk.compressed_offset,
                chunk.compressed_length,
            )?;
            let output_limit = compressed_output_limit(chunk, remaining_output)?;
            let payload = reader.take(chunk.compressed_length);
            bounded_copy(ZlibDecoder::new(payload), output_limit, "zlib", writer)?
        }
        ChunkType::Bzlib => {
            seek_to_payload(
                reader,
                input_size,
                chunk.compressed_offset,
                chunk.compressed_length,
            )?;
            let output_limit = compressed_output_limit(chunk, remaining_output)?;
            let payload = reader.take(chunk.compressed_length);
            bounded_copy(BzDecoder::new(payload), output_limit, "bzip2", writer)?
        }
        ChunkType::Lzfse => {
            read_exact_at_into(
                reader,
                input_size,
                chunk.compressed_offset,
                chunk.compressed_length,
                &mut buffers.compressed,
            )?;
            if chunk.sector_count == 0 {
                bail!("LZFSE chunk is missing a declared expanded sector count");
            }
            let output_limit = compressed_output_limit(chunk, remaining_output)?;
            let buffer_len = output_limit
                .checked_add(1)
                .and_then(|size| usize::try_from(size).ok())
                .ok_or_else(|| anyhow!("LZFSE output buffer size overflow"))?;
            buffers.decoded.clear();
            buffers
                .decoded
                .try_reserve(buffer_len)
                .context("unable to reserve LZFSE output buffer")?;
            buffers.decoded.resize(buffer_len, 0);
            let decoded_len = lzfse::decode_buffer(&buffers.compressed, &mut buffers.decoded)
                .map_err(|error| anyhow!("unable to LZFSE-decompress chunk: {error:?}"))?;
            let decoded_len_u64 =
                u64::try_from(decoded_len).context("LZFSE decoded length does not fit u64")?;
            if decoded_len_u64 > output_limit {
                bail!("LZFSE output exceeds declared size {output_limit}");
            }
            writer
                .write_all(&buffers.decoded[..decoded_len])
                .context("unable to write LZFSE output")?;
            decoded_len_u64
        }
        ChunkType::Zero | ChunkType::Ignore => {
            let raw_len = if chunk.sector_count > 0 {
                chunk
                    .sector_count
                    .checked_mul(SECTOR_SIZE)
                    .ok_or_else(|| anyhow!("chunk sector size overflow"))?
            } else {
                chunk.compressed_length
            };
            if raw_len > remaining_output {
                bail!(
                    "zero-fill chunk length {raw_len} exceeds remaining partition limit {remaining_output}"
                );
            }
            write_zeroes(writer, raw_len)?;
            raw_len
        }
        ChunkType::Comment | ChunkType::Term => 0,
        ChunkType::Adc => {
            bail!("ADC compression is unsupported by the upstream DMG decoder")
        }
    };
    let output_bytes = usize::try_from(written).context("decoded length does not fit usize")?;
    record_decode(chunk.compressed_length, output_bytes);
    Ok(written)
}

#[cfg(any(test, feature = "fuzzing"))]
fn decode_chunk<R: Read + Seek>(
    reader: &mut R,
    input_size: u64,
    chunk: &BlkxChunk,
    remaining_output: u64,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    decode_chunk_to(
        reader,
        input_size,
        chunk,
        remaining_output,
        &mut output,
        &mut DecodeBuffers::default(),
    )?;
    Ok(output)
}

fn partition_output_size(partition: &PartitionRecord) -> Result<u64> {
    let output_size = partition
        .table
        .sector_count
        .checked_mul(SECTOR_SIZE)
        .ok_or_else(|| anyhow!("partition output size overflow"))?;
    if output_size > MAX_PARTITION_BYTES {
        bail!("partition output exceeds limit {MAX_PARTITION_BYTES}");
    }
    Ok(output_size)
}

fn write_partition_to<W: Write>(
    path: &Path,
    partition: &PartitionRecord,
    writer: &mut W,
) -> Result<u64> {
    let file =
        File::open(path).with_context(|| format!("unable to open DMG: {}", path.display()))?;
    let input_size = file
        .metadata()
        .with_context(|| format!("unable to stat DMG: {}", path.display()))?
        .len();
    let mut reader = BufReader::new(file);
    let expected = partition_output_size(partition)?;
    let mut written = 0_u64;
    let mut buffers = DecodeBuffers::default();

    for chunk in &partition.table.chunks {
        let remaining = expected
            .checked_sub(written)
            .ok_or_else(|| anyhow!("partition output exceeds declared size {expected}"))?;
        let chunk_written = decode_chunk_to(
            &mut reader,
            input_size,
            chunk,
            remaining,
            writer,
            &mut buffers,
        )?;
        written = written
            .checked_add(chunk_written)
            .ok_or_else(|| anyhow!("partition output size overflow"))?;
    }

    if written != expected {
        bail!("partition produced {written} bytes; expected {expected}");
    }
    Ok(written)
}

fn read_partition_bytes(path: &Path, partition: &PartitionRecord) -> Result<Vec<u8>> {
    let expected = partition_output_size(partition)?;
    let capacity = usize::try_from(expected).context("partition size does not fit usize")?;
    let mut out = Vec::new();
    out.try_reserve_exact(capacity)
        .context("unable to reserve partition output buffer")?;
    write_partition_to(path, partition, &mut out)?;
    Ok(out)
}

#[derive(Debug)]
struct PartitionSpan {
    chunk_index: usize,
    start: u64,
    end: u64,
}

struct PartitionReader<'a> {
    reader: BufReader<File>,
    input_size: u64,
    chunks: &'a [BlkxChunk],
    spans: Vec<PartitionSpan>,
    size: u64,
    position: u64,
    cached_span: Option<usize>,
    cache: Vec<u8>,
    decode_buffers: DecodeBuffers,
}

impl<'a> PartitionReader<'a> {
    fn open(path: &Path, partition: &'a PartitionRecord) -> Result<Self> {
        let file =
            File::open(path).with_context(|| format!("unable to open DMG: {}", path.display()))?;
        let input_size = file
            .metadata()
            .with_context(|| format!("unable to stat DMG: {}", path.display()))?
            .len();
        let size = partition_output_size(partition)?;
        let mut spans = Vec::with_capacity(partition.table.chunks.len());
        for (chunk_index, chunk) in partition.table.chunks.iter().enumerate() {
            if matches!(chunk.ty(), Some(ChunkType::Comment | ChunkType::Term)) {
                continue;
            }
            let start = chunk
                .sector_number
                .checked_mul(SECTOR_SIZE)
                .ok_or_else(|| anyhow!("partition chunk byte offset overflow"))?;
            let length = chunk
                .sector_count
                .checked_mul(SECTOR_SIZE)
                .ok_or_else(|| anyhow!("partition chunk byte length overflow"))?;
            let end = start
                .checked_add(length)
                .ok_or_else(|| anyhow!("partition chunk byte range overflow"))?;
            if end > size {
                bail!("partition chunk range {start}..{end} exceeds partition size {size}");
            }
            spans.push(PartitionSpan {
                chunk_index,
                start,
                end,
            });
        }

        Ok(Self {
            reader: BufReader::new(file),
            input_size,
            chunks: &partition.table.chunks,
            spans,
            size,
            position: 0,
            cached_span: None,
            cache: Vec::new(),
            decode_buffers: DecodeBuffers::default(),
        })
    }

    fn span_at(&self, position: u64) -> Option<usize> {
        self.spans
            .binary_search_by(|span| {
                if position < span.start {
                    std::cmp::Ordering::Greater
                } else if position >= span.end {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .ok()
    }

    fn load_span(&mut self, span_index: usize) -> io::Result<()> {
        if self.cached_span == Some(span_index) {
            return Ok(());
        }
        let span = self.spans.get(span_index).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "partition span index is out of range",
            )
        })?;
        let chunk = self.chunks.get(span.chunk_index).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "partition chunk index is out of range",
            )
        })?;
        let expected = span.end - span.start;
        let capacity = usize::try_from(expected).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "partition chunk size does not fit usize",
            )
        })?;
        self.cache.clear();
        self.cache
            .try_reserve(capacity)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let written = decode_chunk_to(
            &mut self.reader,
            self.input_size,
            chunk,
            expected,
            &mut self.cache,
            &mut self.decode_buffers,
        )
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        if written != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("partition chunk produced {written} bytes; expected {expected}"),
            ));
        }
        self.cached_span = Some(span_index);
        Ok(())
    }
}

impl Read for PartitionReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() || self.position >= self.size {
            return Ok(0);
        }

        let mut total = 0_usize;
        while total < buffer.len() && self.position < self.size {
            let span_index = self.span_at(self.position).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("partition has no chunk covering byte {}", self.position),
                )
            })?;
            let span = &self.spans[span_index];
            let within = usize::try_from(self.position - span.start).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "partition chunk offset does not fit usize",
                )
            })?;
            let available = usize::try_from(span.end - self.position).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "partition chunk remaining size does not fit usize",
                )
            })?;
            let count = available.min(buffer.len() - total);
            let chunk_type = self.chunks[span.chunk_index].ty();

            if matches!(chunk_type, Some(ChunkType::Zero | ChunkType::Ignore)) {
                buffer[total..total + count].fill(0);
                record_decode(0, count);
            } else {
                self.load_span(span_index)?;
                let end = within.checked_add(count).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "partition cache range overflow")
                })?;
                let source = self.cache.get(within..end).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "partition cache range is out of bounds",
                    )
                })?;
                buffer[total..total + count].copy_from_slice(source);
                record_staging_copy(count);
            }

            total += count;
            self.position = self
                .position
                .checked_add(count as u64)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "position overflow"))?;
        }
        Ok(total)
    }
}

impl Seek for PartitionReader<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let resolved = match position {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.size) + i128::from(offset),
        };
        if !(0..=i128::from(u64::MAX)).contains(&resolved) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek outside the partition address space",
            ));
        }
        self.position = resolved as u64;
        Ok(self.position)
    }
}

impl Write for PartitionReader<'_> {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "DMG partition reader is read-only",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn data_fork_checksum(path: &Path, koly: &KolyTrailer) -> Result<u32> {
    let file =
        File::open(path).with_context(|| format!("unable to open DMG: {}", path.display()))?;
    let file_size = file
        .metadata()
        .with_context(|| format!("unable to stat DMG: {}", path.display()))?
        .len();
    validate_file_range(
        file_size,
        koly.data_fork_offset,
        koly.data_fork_length,
        "data fork",
    )?;
    let mut reader = BufReader::new(file);

    reader
        .seek(SeekFrom::Start(koly.data_fork_offset))
        .context("unable to seek to data fork")?;

    let mut hasher = Hasher::new();
    let mut limited = (&mut reader).take(koly.data_fork_length);
    let mut buffer = [0_u8; 128 * 1024];
    let mut total_read = 0_u64;

    loop {
        let read = limited
            .read(&mut buffer)
            .context("unable to read data fork")?;
        if read == 0 {
            break;
        }
        total_read = total_read
            .checked_add(u64::try_from(read).context("checksum read length does not fit u64")?)
            .ok_or_else(|| anyhow!("checksum read length overflow"))?;
        hasher.update(&buffer[..read]);
    }

    if total_read != koly.data_fork_length {
        bail!(
            "data fork ended after {total_read} bytes; expected {}",
            koly.data_fork_length
        );
    }

    Ok(hasher.finalize())
}

fn truncate_metadata_text(text: &str) -> String {
    let mut characters = text.chars();
    let mut truncated: String = characters.by_ref().take(MAX_METADATA_VALUE_CHARS).collect();
    if characters.next().is_some() {
        truncated.push('…');
    }
    truncated
}

fn value_to_short_string(value: &Value) -> String {
    match value {
        Value::Boolean(v) => v.to_string(),
        Value::Data(bytes) => format!("<{} bytes>", bytes.len()),
        Value::Date(date) => format!("{date:?}"),
        Value::Real(real) => real.to_string(),
        Value::Integer(int) => format!("{int:?}"),
        Value::String(s) => truncate_metadata_text(s),
        Value::Array(values) => {
            const MAX_PREVIEW_ITEMS: usize = 16;
            let parts: Vec<String> = values
                .iter()
                .take(MAX_PREVIEW_ITEMS)
                .map(|item| match item {
                    Value::Array(_) => "<array>".to_string(),
                    other => value_to_short_string(other),
                })
                .collect();
            let suffix = if values.len() > MAX_PREVIEW_ITEMS {
                ", …"
            } else {
                ""
            };
            truncate_metadata_text(&format!("[{}{suffix}]", parts.join(", ")))
        }
        Value::Dictionary(_) => "<dictionary>".to_string(),
        other => format!("{other:?}"),
    }
}

fn plist_to_json(value: &Value) -> JsonValue {
    match value {
        Value::Array(values) => JsonValue::Array(values.iter().map(plist_to_json).collect()),
        Value::Dictionary(dict) => {
            let mut object = serde_json::Map::with_capacity(dict.len());
            for (key, val) in dict {
                object.insert(key.clone(), plist_to_json(val));
            }
            JsonValue::Object(object)
        }
        Value::Boolean(v) => JsonValue::Bool(*v),
        Value::Data(bytes) => {
            let preview_len = bytes.len().min(64);
            json!({
                "__type__": "data",
                "size": bytes.len(),
                "preview_base64": BASE64_STANDARD.encode(&bytes[..preview_len]),
            })
        }
        Value::Date(date) => json!({
            "__type__": "date",
            "value": format!("{date:?}"),
        }),
        Value::Real(real) => json!(real),
        Value::Integer(int) => JsonValue::String(format!("{int:?}")),
        Value::String(text) => JsonValue::String(text.clone()),
        other => JsonValue::String(format!("{other:?}")),
    }
}

fn push_candidate(
    target: &mut Vec<MetadataCandidate>,
    path: &[String],
    key: &str,
    value: &Value,
    count: &mut usize,
) {
    if *count >= MAX_METADATA_CANDIDATES {
        return;
    }
    let rendered = value_to_short_string(value);
    target.push(MetadataCandidate {
        path: truncate_metadata_text(&path.join(".")),
        key: truncate_metadata_text(key),
        value: rendered,
    });
    *count += 1;
}

fn collect_metadata_candidates(value: &Value) -> MetadataCandidates {
    fn walk(
        value: &Value,
        path: &mut Vec<String>,
        out: &mut MetadataCandidates,
        count: &mut usize,
    ) {
        if *count >= MAX_METADATA_CANDIDATES {
            return;
        }
        match value {
            Value::Dictionary(dict) => {
                for (key, child) in dict {
                    path.push(key.clone());
                    let key_lower = key.to_ascii_lowercase();

                    if key_lower.contains("date")
                        || key_lower.contains("time")
                        || key_lower.contains("created")
                        || key_lower.contains("creation")
                        || key_lower.contains("modified")
                        || key_lower.contains("timestamp")
                    {
                        push_candidate(&mut out.creation_dates, path, key, child, count);
                    }

                    if key_lower.contains("author")
                        || key_lower.contains("creator")
                        || key_lower.contains("owner")
                        || key_lower.contains("publisher")
                    {
                        push_candidate(&mut out.authors, path, key, child, count);
                    }

                    if key_lower.contains("application")
                        || key_lower.contains("app")
                        || key_lower.contains("software")
                        || key_lower.contains("program")
                        || key_lower.contains("generator")
                        || key_lower.contains("creator")
                    {
                        push_candidate(&mut out.creation_applications, path, key, child, count);
                    }

                    walk(child, path, out, count);
                    let _ = path.pop();
                }
            }
            Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    path.push(index.to_string());
                    walk(child, path, out, count);
                    let _ = path.pop();
                }
            }
            _ => {}
        }
    }

    let mut out = MetadataCandidates::default();
    let mut path = Vec::new();
    let mut count = 0;
    walk(value, &mut path, &mut out, &mut count);
    out
}

fn chunk_type_name(chunk_type: Option<ChunkType>) -> Option<String> {
    chunk_type.map(|kind| {
        match kind {
            ChunkType::Zero => "Zero",
            ChunkType::Raw => "Raw",
            ChunkType::Ignore => "Ignore",
            ChunkType::Comment => "Comment",
            ChunkType::Adc => "Adc",
            ChunkType::Zlib => "Zlib",
            ChunkType::Bzlib => "Bzlib",
            ChunkType::Lzfse => "Lzfse",
            ChunkType::Term => "Term",
        }
        .to_string()
    })
}

fn to_koly_info(koly: &KolyTrailer) -> KolyInfo {
    KolyInfo {
        version: koly.version,
        flags: koly.flags,
        running_data_fork_offset: koly.running_data_fork_offset,
        data_fork_offset: koly.data_fork_offset,
        data_fork_length: koly.data_fork_length,
        resource_fork_offset: koly.resource_fork_offset,
        resource_fork_length: koly.resource_fork_length,
        segment_number: koly.segment_number,
        segment_count: koly.segment_count,
        segment_id_hex: hex_encode(&koly.segment_id),
        plist_offset: koly.plist_offset,
        plist_length: koly.plist_length,
        code_signature_offset: koly.code_signature_offset,
        code_signature_size: koly.code_signature_size,
        image_variant: koly.image_variant,
        sector_count: koly.sector_count,
        data_fork_digest_crc32: u32::from(koly.data_fork_digest),
        main_digest_crc32: u32::from(koly.main_digest),
    }
}

fn to_partition_info(partition: &PartitionRecord) -> PartitionInfo {
    let chunks = partition
        .table
        .chunks
        .iter()
        .map(|chunk| BlkxChunkInfo {
            chunk_type: chunk_type_name(chunk.ty()),
            raw_type: chunk.r#type,
            comment: chunk.comment,
            sector_number: chunk.sector_number,
            sector_count: chunk.sector_count,
            compressed_offset: chunk.compressed_offset,
            compressed_length: chunk.compressed_length,
        })
        .collect::<Vec<_>>();

    PartitionInfo {
        index: partition.index,
        id: partition.id.clone(),
        name: partition.name.clone(),
        cfname: partition.cfname.clone(),
        attributes: partition.attributes.clone(),
        table: BlkxTableInfo {
            version: partition.table.version,
            sector_number: partition.table.sector_number,
            sector_count: partition.table.sector_count,
            data_offset: partition.table.data_offset,
            buffers_needed: partition.table.buffers_needed,
            block_descriptors: partition.table.block_descriptors,
            checksum_crc32: u32::from(partition.table.checksum),
            chunk_count: chunks.len(),
            chunks,
        },
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'a' + (value - 10)) as char,
    }
}

fn logical_block_size_bytes(lb: LogicalBlockSize) -> u64 {
    match lb {
        LogicalBlockSize::Lb512 => 512,
        LogicalBlockSize::Lb4096 => 4096,
    }
}

fn gpt_header_to_info(header: &gpt::header::Header) -> GptHeaderInfo {
    GptHeaderInfo {
        signature: header.signature.clone(),
        revision_major: header.revision.0,
        revision_minor: header.revision.1,
        header_size: header.header_size_le,
        crc32: header.crc32,
        current_lba: header.current_lba,
        backup_lba: header.backup_lba,
        first_usable_lba: header.first_usable,
        last_usable_lba: header.last_usable,
        disk_guid: header.disk_guid.to_string(),
        partition_table_start_lba: header.part_start,
        partition_count: header.num_parts,
        partition_entry_size: header.part_size,
        partition_table_crc32: header.crc32_parts,
    }
}

fn read_backup_gpt_header(
    bytes: &[u8],
    block_size: LogicalBlockSize,
) -> Option<gpt::header::Header> {
    let block_bytes = logical_block_size_bytes(block_size);
    let input_size = u64::try_from(bytes.len()).ok()?;
    if input_size < block_bytes.checked_mul(3)? {
        return None;
    }

    // Match `gpt`'s backup-header lookup, including its behavior for an input
    // whose size is not an exact multiple of the logical block size. The public
    // arbitrary-device helper always reads LBA 1, so present the backup block as
    // the second block of a subslice.
    let backup_lba = input_size.saturating_sub(block_bytes) / block_bytes;
    let backup_offset = backup_lba.checked_mul(block_bytes)?;
    let subslice_start = backup_offset.checked_sub(block_bytes)?;
    let subslice_start = usize::try_from(subslice_start).ok()?;
    let mut device = Cursor::new(bytes.get(subslice_start..)?);
    gpt::header::read_header_from_arbitrary_device(&mut device, block_size).ok()
}

fn validate_gpt_header(
    bytes: &[u8],
    block_size: LogicalBlockSize,
    header_name: &str,
    header: &gpt::header::Header,
) -> Result<()> {
    if header.part_size != 128 {
        bail!(
            "{header_name} GPT partition entry size {} is unsupported; expected 128",
            header.part_size
        );
    }
    if header.num_parts > MAX_GPT_PARTITIONS {
        bail!(
            "{header_name} GPT partition count {} exceeds limit {MAX_GPT_PARTITIONS}",
            header.num_parts
        );
    }

    let block_bytes = logical_block_size_bytes(block_size);
    let table_offset = header
        .part_start
        .checked_mul(block_bytes)
        .ok_or_else(|| anyhow!("{header_name} GPT partition table offset overflow"))?;
    let table_length = u64::from(header.num_parts)
        .checked_mul(u64::from(header.part_size))
        .ok_or_else(|| anyhow!("{header_name} GPT partition table length overflow"))?;
    let input_size = u64::try_from(bytes.len()).context("GPT input size exceeds u64")?;
    validate_file_range(
        input_size,
        table_offset,
        table_length,
        &format!("{header_name} GPT partition table"),
    )?;

    let table_start = usize::try_from(table_offset).context("GPT table offset exceeds usize")?;
    let table_end = usize::try_from(
        table_offset
            .checked_add(table_length)
            .ok_or_else(|| anyhow!("{header_name} GPT partition table end overflow"))?,
    )
    .context("GPT table end exceeds usize")?;
    let computed_crc32 = crc32fast::hash(
        bytes
            .get(table_start..table_end)
            .ok_or_else(|| anyhow!("{header_name} GPT partition table is out of bounds"))?,
    );
    if computed_crc32 != header.crc32_parts {
        bail!(
            "{header_name} GPT partition table CRC32 mismatch: declared {:#010x}, computed {computed_crc32:#010x}",
            header.crc32_parts
        );
    }

    Ok(())
}

fn preflight_gpt(bytes: &[u8], block_size: LogicalBlockSize) -> Result<()> {
    let mut primary_device = Cursor::new(bytes);
    let primary =
        gpt::header::read_header_from_arbitrary_device(&mut primary_device, block_size).ok();
    let backup = read_backup_gpt_header(bytes, block_size);

    if let Some(header) = primary.as_ref() {
        validate_gpt_header(bytes, block_size, "primary", header)
    } else if let Some(header) = backup.as_ref() {
        validate_gpt_header(bytes, block_size, "backup", header)
    } else {
        // The dependency will return its detailed invalid-header error without
        // attempting to read a partition array.
        Ok(())
    }
}

fn parse_gpt_from_bytes(bytes: &[u8]) -> std::result::Result<ParsedGpt, Vec<String>> {
    let mut errors = Vec::new();
    let block_sizes = [LogicalBlockSize::Lb512, LogicalBlockSize::Lb4096];

    for block_size in block_sizes {
        let lb_bytes = logical_block_size_bytes(block_size);
        if let Err(error) = preflight_gpt(bytes, block_size) {
            errors.push(format!("LBA {lb_bytes}: {error}"));
            continue;
        }

        let device = ReadOnlyDevice::new(bytes);
        let config = GptConfig::new()
            .writable(false)
            .logical_block_size(block_size)
            .only_valid_headers(false);

        let opened = catch_dependency_panic("GPT parsing", || {
            config.open_from_device(device).map_err(anyhow::Error::from)
        });
        match opened {
            Ok(disk) => {
                let primary_header = disk.primary_header().ok().map(gpt_header_to_info);
                let backup_header = disk.backup_header().ok().map(gpt_header_to_info);

                let primary_header_valid = primary_header.is_some();
                let backup_header_valid = backup_header.is_some();

                let mut partitions = Vec::with_capacity(disk.partitions().len());
                for (entry_index, partition) in disk.partitions() {
                    let sector_count = partition.sectors_len().unwrap_or(0);
                    let byte_size = partition.bytes_len(block_size).unwrap_or(0);
                    partitions.push(GptPartitionInfo {
                        entry_index: *entry_index,
                        type_guid: partition.part_type_guid.guid.to_string(),
                        type_os: format!("{:?}", partition.part_type_guid.os),
                        unique_guid: partition.part_guid.to_string(),
                        name: partition.name.clone(),
                        first_lba: partition.first_lba,
                        last_lba: partition.last_lba,
                        sector_count,
                        byte_size,
                        flags: partition.flags,
                    });
                }

                return Ok(ParsedGpt {
                    logical_block_size: lb_bytes,
                    disk_guid: disk.guid().to_string(),
                    primary_header,
                    backup_header,
                    validation: GptValidation {
                        primary_header_valid,
                        backup_header_valid,
                        using_backup_as_fallback: !primary_header_valid && backup_header_valid,
                    },
                    partitions,
                });
            }
            Err(error) => {
                errors.push(format!("LBA {lb_bytes}: {error}"));
            }
        }
    }

    Err(errors)
}

fn fat_type_name(fat_type: FatType) -> &'static str {
    match fat_type {
        FatType::Fat12 => "fat12",
        FatType::Fat16 => "fat16",
        FatType::Fat32 => "fat32",
    }
}

fn open_fat_filesystem<'a>(
    path: &Path,
    partition: &'a PartitionRecord,
) -> Result<FileSystem<BudgetedIo<PartitionReader<'a>>>> {
    let reader = PartitionReader::open(path, partition)?;
    FileSystem::new(BudgetedIo::new(reader), FsOptions::new())
        .context("unable to open FAT filesystem from partition data")
}

fn detect_fat_filesystems(
    path: &Path,
    partitions: &[PartitionRecord],
) -> (Vec<FatFilesystemMetadata>, Vec<String>) {
    let mut filesystems = Vec::new();
    let mut errors = Vec::new();

    for partition in partitions {
        let detected = catch_dependency_panic("FAT filesystem detection", || {
            let fs = match open_fat_filesystem(path, partition) {
                Ok(fs) => fs,
                Err(_) => return Ok(None),
            };

            let stats = fs
                .stats()
                .context("unable to read FAT filesystem statistics")?;
            let root_volume_label = fs
                .read_volume_label_from_root_dir()
                .context("unable to read FAT root volume label")?;

            Ok(Some(FatFilesystemMetadata {
                partition_index: partition.index,
                partition_name: partition.name.clone(),
                fat_type: fat_type_name(fs.fat_type()).to_string(),
                volume_id: fs.volume_id(),
                volume_label: fs.volume_label(),
                root_volume_label,
                cluster_size: Some(stats.cluster_size()),
                total_clusters: Some(stats.total_clusters()),
                free_clusters: Some(stats.free_clusters()),
            }))
        });

        match detected {
            Ok(Some(filesystem)) => filesystems.push(filesystem),
            Ok(None) => {}
            Err(error) => errors.push(format!(
                "partition {} ({}) FAT inspection failed: {error}",
                partition.index, partition.name
            )),
        }
    }

    (filesystems, errors)
}

fn panic_payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

fn catch_dependency_panic<T>(operation: &str, callback: impl FnOnce() -> Result<T>) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(callback)) {
        Ok(result) => result,
        Err(payload) => bail!(
            "{operation} rejected malformed input after an upstream parser panic: {}",
            panic_payload_message(payload.as_ref())
        ),
    }
}

fn validate_apple_partition_size(size: u64) -> Result<()> {
    if size > MAX_PARTITION_BYTES {
        bail!("filesystem image size {size} exceeds limit {MAX_PARTITION_BYTES}");
    }
    Ok(())
}

fn reader_size<R: Seek>(reader: &mut R) -> Result<u64> {
    let original = reader
        .stream_position()
        .context("unable to read filesystem image position")?;
    let size = reader
        .seek(SeekFrom::End(0))
        .context("unable to determine filesystem image size")?;
    reader
        .seek(SeekFrom::Start(original))
        .context("unable to restore filesystem image position")?;
    Ok(size)
}

fn validate_hfs_reader<R: Read + Seek>(reader: &mut R) -> Result<()> {
    let file_size = reader_size(reader)?;
    validate_apple_partition_size(file_size)?;
    reader
        .seek(SeekFrom::Start(HFS_VOLUME_HEADER_OFFSET))
        .context("unable to seek to HFS+ volume header")?;
    let mut prefix = [0_u8; HFS_VOLUME_HEADER_PREFIX_SIZE];
    reader
        .read_exact(&mut prefix)
        .context("unable to read HFS+ volume header")?;

    if !matches!(&prefix[..2], b"H+" | b"HX") {
        bail!("filesystem image has no HFS+/HFSX signature");
    }
    let block_size = u32::from_be_bytes(
        prefix[40..44]
            .try_into()
            .expect("fixed-size HFS+ block-size field"),
    );
    if !(HFS_MIN_ALLOCATION_BLOCK_BYTES..=MAX_HFS_ALLOCATION_BLOCK_BYTES).contains(&block_size)
        || !block_size.is_power_of_two()
    {
        bail!(
            "HFS+ allocation block size {block_size} is outside the supported power-of-two range {HFS_MIN_ALLOCATION_BLOCK_BYTES}..={MAX_HFS_ALLOCATION_BLOCK_BYTES}"
        );
    }

    let total_blocks = u32::from_be_bytes(
        prefix[44..48]
            .try_into()
            .expect("fixed-size HFS+ total-blocks field"),
    );
    let volume_size = u64::from(block_size)
        .checked_mul(u64::from(total_blocks))
        .ok_or_else(|| anyhow!("HFS+ volume size overflow"))?;
    if total_blocks == 0 || volume_size > file_size {
        bail!(
            "HFS+ volume declares {volume_size} bytes in {total_blocks} blocks, beyond image size {file_size}"
        );
    }

    reader
        .seek(SeekFrom::Start(0))
        .context("unable to rewind HFS+ image")?;
    Ok(())
}

fn validate_apfs_block_size(block_size: u32) -> Result<()> {
    if !(APFS_MIN_BLOCK_BYTES..=APFS_MAX_BLOCK_BYTES).contains(&block_size)
        || !block_size.is_power_of_two()
    {
        bail!(
            "APFS block size {block_size} is outside the supported power-of-two range {APFS_MIN_BLOCK_BYTES}..={APFS_MAX_BLOCK_BYTES}"
        );
    }
    Ok(())
}

fn validate_apfs_superblock(nxsb: &apfs::superblock::NxSuperblock, file_size: u64) -> Result<()> {
    validate_apfs_block_size(nxsb.block_size)?;
    let container_size = nxsb
        .block_count
        .checked_mul(u64::from(nxsb.block_size))
        .ok_or_else(|| anyhow!("APFS container size overflow"))?;
    if nxsb.block_count == 0 || container_size > file_size {
        bail!(
            "APFS container declares {container_size} bytes in {} blocks, beyond image size {file_size}",
            nxsb.block_count
        );
    }
    if nxsb.xp_desc_blocks > MAX_APFS_CHECKPOINT_BLOCKS {
        bail!(
            "APFS checkpoint descriptor count {} exceeds limit {MAX_APFS_CHECKPOINT_BLOCKS}",
            nxsb.xp_desc_blocks
        );
    }
    let checkpoint_end = nxsb
        .xp_desc_base
        .checked_add(u64::from(nxsb.xp_desc_blocks))
        .ok_or_else(|| anyhow!("APFS checkpoint descriptor range overflow"))?;
    if checkpoint_end > nxsb.block_count {
        bail!(
            "APFS checkpoint descriptor range ends at block {checkpoint_end}, beyond container block count {}",
            nxsb.block_count
        );
    }
    Ok(())
}

fn validate_apfs_reader<R: Read + Seek>(reader: &mut R) -> Result<()> {
    let file_size = reader_size(reader)?;
    validate_apple_partition_size(file_size)?;
    reader
        .seek(SeekFrom::Start(0))
        .context("unable to seek to APFS container superblock")?;
    let mut prefix = [0_u8; APFS_SUPERBLOCK_PREFIX_SIZE];
    reader
        .read_exact(&mut prefix)
        .context("unable to read APFS container superblock prefix")?;
    if &prefix[32..36] != b"NXSB" {
        bail!("filesystem image has no APFS container signature");
    }
    let block_size = u32::from_le_bytes(
        prefix[36..40]
            .try_into()
            .expect("fixed-size APFS block-size field"),
    );
    validate_apfs_block_size(block_size)?;
    let block_count = u64::from_le_bytes(
        prefix[40..48]
            .try_into()
            .expect("fixed-size APFS block-count field"),
    );
    let container_size = block_count
        .checked_mul(u64::from(block_size))
        .ok_or_else(|| anyhow!("APFS container size overflow"))?;
    if block_count == 0 || container_size > file_size {
        bail!(
            "APFS container declares {container_size} bytes in {block_count} blocks, beyond image size {file_size}"
        );
    }

    let nxsb = apfs::superblock::read_nxsb(reader)
        .context("unable to validate APFS container superblock")?;
    validate_apfs_superblock(&nxsb, file_size)?;
    let latest = apfs::superblock::find_latest_nxsb(reader, &nxsb)
        .context("unable to validate APFS checkpoint superblocks")?;
    validate_apfs_superblock(&latest, file_size)?;
    reader
        .seek(SeekFrom::Start(0))
        .context("unable to rewind APFS image")?;
    Ok(())
}

fn open_hfs_volume<R: Read + Seek>(mut reader: R) -> Result<hfsplus::HfsVolume<R>> {
    validate_hfs_reader(&mut reader)?;
    hfsplus::HfsVolume::open(reader).context("unable to parse HFS+ filesystem")
}

fn open_apfs_volume<R: Read + Seek>(mut reader: R) -> Result<apfs::ApfsVolume<R>> {
    validate_apfs_reader(&mut reader)?;
    apfs::ApfsVolume::open(reader).context("unable to parse APFS filesystem")
}

fn verify_streaming_data_fork_checksum(path: &Path, koly: &KolyTrailer) -> Result<()> {
    let declared = u32::from(koly.data_fork_digest);
    if koly.data_fork_digest.r#type != 2 || declared == 0 {
        return Ok(());
    }
    let computed = data_fork_checksum(path, koly)?;
    if computed != declared {
        bail!("DMG data-fork CRC32 mismatch: declared {declared:08x}, computed {computed:08x}");
    }
    Ok(())
}

fn open_udif_apple_filesystem(path: &Path) -> Result<AppleFsHandle> {
    let parsed =
        parse_dmg(path).context("DMG boundary validation failed before filesystem extraction")?;
    verify_streaming_data_fork_checksum(path, &parsed.koly)?;

    let mut archive = UdifDmgArchive::open_with_options(
        path,
        UdifDmgReaderOptions {
            // udif's verifier materializes the entire attacker-controlled data
            // fork. The repository performs the equivalent CRC32 check above
            // with a fixed-size streaming buffer.
            verify_checksums: false,
        },
    )
    .with_context(|| {
        format!(
            "unable to open DMG filesystem container: {}",
            path.display()
        )
    })?;
    let partitions = archive.partitions();
    let partition = partitions
        .iter()
        .filter(|partition| partition.partition_type.is_hfs_compatible())
        .max_by_key(|partition| partition.size)
        .or_else(|| {
            partitions
                .iter()
                .filter(|partition| partition.partition_type == UdifPartitionType::Apfs)
                .max_by_key(|partition| partition.size)
        })
        .ok_or_else(|| anyhow!("DMG has no HFS+/HFSX/APFS partition"))?
        .clone();

    validate_apple_partition_size(partition.size)?;

    let mut temporary = tempfile().context("unable to create temporary filesystem partition")?;
    let (reported, written) = {
        let mut limited = LimitedWriter::new(&mut temporary, MAX_PARTITION_BYTES);
        let reported = archive
            .extract_partition_to(partition.id, &mut limited)
            .with_context(|| format!("unable to extract DMG partition {}", partition.id))?;
        limited
            .flush()
            .context("unable to flush temporary filesystem partition")?;
        (reported, limited.written())
    };
    if reported != written {
        bail!("DMG filesystem extractor reported {reported} bytes but wrote {written}");
    }
    record_decode(
        partition.compressed_size,
        usize::try_from(written).unwrap_or(usize::MAX),
    );

    temporary
        .seek(SeekFrom::Start(0))
        .context("unable to rewind temporary filesystem partition")?;
    let reader = BudgetedIo::new(BufReader::new(temporary));

    match partition.partition_type {
        UdifPartitionType::Hfs | UdifPartitionType::Hfsx => {
            let volume =
                open_hfs_volume(reader).context("unable to parse extracted HFS+ filesystem")?;
            Ok(AppleFsHandle::Hfs(Box::new(volume)))
        }
        UdifPartitionType::Apfs => {
            let volume =
                open_apfs_volume(reader).context("unable to parse extracted APFS filesystem")?;
            Ok(AppleFsHandle::Apfs(Box::new(volume)))
        }
        UdifPartitionType::Other => bail!("selected DMG partition has no supported filesystem"),
    }
}

fn open_apple_filesystem(path: &Path) -> Result<AppleFsHandle> {
    let mut errors = Vec::new();

    match open_udif_apple_filesystem(path) {
        Ok(filesystem) => return Ok(filesystem),
        Err(error) => errors.push(format!("DMG filesystem detection failed: {error}")),
    }

    match File::open(path) {
        Ok(file) => {
            let reader = BudgetedIo::new(BufReader::new(file));
            match open_hfs_volume(reader) {
                Ok(volume) => return Ok(AppleFsHandle::Hfs(Box::new(volume))),
                Err(error) => errors.push(format!("raw HFS+ parse failed: {error}")),
            }
        }
        Err(error) => {
            errors.push(format!("unable to open image for raw HFS+ parse: {error}"));
        }
    }

    match File::open(path) {
        Ok(file) => {
            let reader = BudgetedIo::new(BufReader::new(file));
            match open_apfs_volume(reader) {
                Ok(volume) => return Ok(AppleFsHandle::Apfs(Box::new(volume))),
                Err(error) => errors.push(format!("raw APFS parse failed: {error}")),
            }
        }
        Err(error) => {
            errors.push(format!("unable to open image for raw APFS parse: {error}"));
        }
    }

    bail!(
        "unable to detect HFS+/APFS filesystem from image {}: {}",
        path.display(),
        errors.join(" | ")
    );
}

fn apple_metadata_from_handle(handle: &mut AppleFsHandle) -> AppleFilesystemMetadata {
    match handle {
        AppleFsHandle::Hfs(hfs) => {
            let header = hfs.volume_header();
            let block_size = header.block_size;
            let file_count = u64::from(header.file_count);
            let directory_count = u64::from(header.folder_count);
            let total_blocks = header.total_blocks;
            let free_blocks = header.free_blocks;
            let version = header.version;
            let is_hfsx = header.is_hfsx;
            let volume_create_time = i64::from(header.create_date);
            let volume_modify_time = i64::from(header.modify_date);
            let (root_create_time, root_modify_time) = match hfs.stat("/") {
                Ok(root) => (
                    Some(i64::from(root.create_date)),
                    Some(i64::from(root.modify_date)),
                ),
                Err(_) => (None, None),
            };

            AppleFilesystemMetadata {
                fs_type: "hfsplus".to_string(),
                block_size,
                file_count,
                directory_count,
                volume_name: None,
                symlink_count: None,
                total_blocks: Some(total_blocks),
                free_blocks: Some(free_blocks),
                version: Some(version),
                is_hfsx: Some(is_hfsx),
                volume_create_time: Some(volume_create_time),
                volume_modify_time: Some(volume_modify_time),
                root_create_time,
                root_modify_time,
            }
        }
        AppleFsHandle::Apfs(apfs) => {
            let info = apfs.volume_info();
            let block_size = info.block_size;
            let file_count = info.num_files;
            let directory_count = info.num_directories;
            let volume_name = info.name.clone();
            let symlink_count = info.num_symlinks;
            let (root_create_time, root_modify_time) = match apfs.stat("/") {
                Ok(root) => (Some(root.create_time), Some(root.modify_time)),
                Err(_) => (None, None),
            };

            AppleFilesystemMetadata {
                fs_type: "apfs".to_string(),
                block_size,
                file_count,
                directory_count,
                volume_name: Some(volume_name),
                symlink_count: Some(symlink_count),
                total_blocks: None,
                free_blocks: None,
                version: None,
                is_hfsx: None,
                volume_create_time: None,
                volume_modify_time: None,
                root_create_time,
                root_modify_time,
            }
        }
    }
}

fn inspect_apple_filesystem_metadata(path: &Path) -> Result<AppleFilesystemMetadata> {
    catch_dependency_panic("Apple filesystem metadata inspection", || {
        let mut handle = open_apple_filesystem(path)?;
        Ok(apple_metadata_from_handle(&mut handle))
    })
}

fn inspect_filesystems_impl(path: &Path) -> Result<FilesystemsInspection> {
    let mut fat_filesystems = Vec::new();
    let mut errors = Vec::new();
    let mut dmg_parse_error = None;

    match parse_dmg(path) {
        Ok(parsed) => {
            let (detected_fat, fat_errors) = detect_fat_filesystems(path, &parsed.partitions);
            fat_filesystems = detected_fat;
            errors.extend(fat_errors);
        }
        Err(error) => {
            dmg_parse_error = Some(error.to_string());
        }
    }

    let apple_filesystem = match inspect_apple_filesystem_metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) => {
            errors.push(error.to_string());
            None
        }
    };

    if apple_filesystem.is_none() && fat_filesystems.is_empty() {
        if let Some(error) = dmg_parse_error {
            errors.push(error);
        }
    }

    Ok(FilesystemsInspection {
        path: path.display().to_string(),
        fat_filesystems,
        apple_filesystem,
        errors,
    })
}

fn normalize_apple_path(path: &str) -> String {
    if path.is_empty() || path == "/" {
        "/".to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn join_apple_entry_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

fn list_apple_entries_impl(path: &Path, directory_path: &str) -> Result<Vec<AppleFsEntry>> {
    catch_dependency_panic("Apple filesystem directory listing", || {
        let normalized_dir = normalize_apple_path(directory_path);
        let mut handle = open_apple_filesystem(path)?;

        let entries = match &mut handle {
            AppleFsHandle::Hfs(hfs) => hfs
                .list_directory(&normalized_dir)
                .with_context(|| format!("unable to list directory: {normalized_dir}"))?
                .into_iter()
                .map(|entry| AppleFsEntry {
                    path: join_apple_entry_path(&normalized_dir, &entry.name),
                    is_dir: matches!(entry.kind, hfsplus::EntryKind::Directory),
                    size: entry.size,
                })
                .collect::<Vec<_>>(),
            AppleFsHandle::Apfs(apfs) => apfs
                .list_directory(&normalized_dir)
                .with_context(|| format!("unable to list directory: {normalized_dir}"))?
                .into_iter()
                .map(|entry| AppleFsEntry {
                    path: join_apple_entry_path(&normalized_dir, &entry.name),
                    is_dir: matches!(entry.kind, apfs::EntryKind::Directory),
                    size: entry.size,
                })
                .collect::<Vec<_>>(),
        };

        if entries.len() > MAX_FILESYSTEM_ENTRIES {
            bail!(
                "directory entry count {} exceeds limit {MAX_FILESYSTEM_ENTRIES}",
                entries.len()
            );
        }
        Ok(entries)
    })
}

struct LimitedWriter<W> {
    inner: W,
    written: u64,
    limit: u64,
}

impl<W> LimitedWriter<W> {
    fn new(inner: W, limit: u64) -> Self {
        Self {
            inner,
            written: 0,
            limit,
        }
    }

    fn written(&self) -> u64 {
        self.written
    }
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let requested = u64::try_from(buffer.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "write length does not fit u64")
        })?;
        let new_total = self
            .written
            .checked_add(requested)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "output length overflow"))?;
        if new_total > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("output exceeds safety limit {} bytes", self.limit),
            ));
        }

        let written = self.inner.write(buffer)?;
        self.written = self
            .written
            .checked_add(u64::try_from(written).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "write count does not fit u64")
            })?)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "output length overflow"))?;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn output_parent(output_path: &Path) -> &Path {
    output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn create_atomic_output(output_path: &Path) -> Result<NamedTempFile> {
    let parent = output_parent(output_path);
    fs::create_dir_all(parent)
        .with_context(|| format!("unable to create output directory: {}", parent.display()))?;
    NamedTempFile::new_in(parent)
        .with_context(|| format!("unable to create temporary output in {}", parent.display()))
}

fn persist_atomic_output(mut temporary: NamedTempFile, output_path: &Path) -> Result<()> {
    temporary.flush().with_context(|| {
        format!(
            "unable to flush temporary output for {}",
            output_path.display()
        )
    })?;
    temporary.as_file().sync_all().with_context(|| {
        format!(
            "unable to synchronize temporary output for {}",
            output_path.display()
        )
    })?;
    temporary
        .persist(output_path)
        .map_err(|error| error.error)
        .with_context(|| format!("unable to atomically replace {}", output_path.display()))?;
    Ok(())
}

fn write_apple_file_to<W: Write>(
    handle: &mut AppleFsHandle,
    normalized_path: &str,
    writer: &mut W,
) -> Result<u64> {
    match handle {
        AppleFsHandle::Hfs(hfs) => hfs
            .read_file_to(normalized_path, writer)
            .with_context(|| format!("unable to read filesystem file: {normalized_path}")),
        AppleFsHandle::Apfs(apfs) => apfs
            .read_file_to(normalized_path, writer)
            .with_context(|| format!("unable to read filesystem file: {normalized_path}")),
    }
}

fn read_apple_file_impl(path: &Path, file_path: &str) -> Result<Vec<u8>> {
    catch_dependency_panic("Apple filesystem file read", || {
        let normalized_path = normalize_apple_path(file_path);
        let mut handle = open_apple_filesystem(path)?;
        let mut output = Vec::new();
        let (reported, written) = {
            let mut limited = LimitedWriter::new(&mut output, MAX_APPLE_FILE_BYTES);
            let reported = write_apple_file_to(&mut handle, &normalized_path, &mut limited)?;
            limited
                .flush()
                .context("unable to finalize filesystem file read")?;
            (reported, limited.written())
        };
        if reported != written {
            bail!("filesystem reported {reported} bytes but wrote {written}");
        }
        Ok(output)
    })
}

fn extract_apple_file_impl(path: &Path, file_path: &str, output_path: &Path) -> Result<u64> {
    let normalized_path = normalize_apple_path(file_path);
    let mut temporary = create_atomic_output(output_path)?;
    let written = catch_dependency_panic("Apple filesystem file extraction", || {
        let mut handle = open_apple_filesystem(path)?;
        let mut limited = LimitedWriter::new(&mut temporary, MAX_APPLE_FILE_BYTES);
        let reported = write_apple_file_to(&mut handle, &normalized_path, &mut limited)?;
        limited
            .flush()
            .context("unable to finalize filesystem file extraction")?;
        if reported != limited.written() {
            bail!(
                "filesystem reported {reported} bytes but wrote {}",
                limited.written()
            );
        }
        Ok(limited.written())
    })?;
    record_sink_write(written);
    persist_atomic_output(temporary, output_path)?;
    Ok(written)
}

fn create_dmg_with_dependency(
    source: &Path,
    output: &Path,
    volume_label: &str,
    total_sectors: u32,
) -> Result<()> {
    catch_dependency_panic("DMG creation", || {
        apple_dmg::create_dmg(source, output, volume_label, total_sectors)
    })
}

fn inspect_impl(path: &Path) -> Result<DmgInfo> {
    let parsed = parse_dmg(path)?;
    let computed = data_fork_checksum(path, &parsed.koly)?;
    let declared_data = u32::from(parsed.koly.data_fork_digest);
    let (fat_filesystems, mut filesystem_errors) = detect_fat_filesystems(path, &parsed.partitions);
    let apple_filesystem = match inspect_apple_filesystem_metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) => {
            filesystem_errors.push(error.to_string());
            None
        }
    };

    Ok(DmgInfo {
        path: path.display().to_string(),
        file_size: parsed.file_size,
        koly: to_koly_info(&parsed.koly),
        checksum: ChecksumInfo {
            declared_data_fork_crc32: declared_data,
            computed_data_fork_crc32: computed,
            data_fork_checksum_matches: declared_data == computed,
            declared_main_crc32: u32::from(parsed.koly.main_digest),
        },
        plist: plist_to_json(&parsed.plist),
        metadata_candidates: collect_metadata_candidates(&parsed.plist),
        partitions: parsed.partitions.iter().map(to_partition_info).collect(),
        fat_filesystems,
        apple_filesystem,
        filesystem_errors,
    })
}

fn inspect_gpt_impl(path: &Path, partition_index: Option<usize>) -> Result<GptInspection> {
    let parsed = parse_dmg(path)?;

    let candidate_indices = match partition_index {
        Some(index) => {
            if index >= parsed.partitions.len() {
                bail!(
                    "partition index {index} out of range; available partitions: {}",
                    parsed.partitions.len()
                );
            }
            vec![index]
        }
        None => (0..parsed.partitions.len()).collect::<Vec<_>>(),
    };

    let mut errors = Vec::new();

    for index in candidate_indices {
        let partition = parsed
            .partitions
            .get(index)
            .ok_or_else(|| anyhow!("partition index {index} out of range"))?;
        let bytes = read_partition_bytes(path, partition)?;

        match parse_gpt_from_bytes(&bytes) {
            Ok(gpt) => {
                return Ok(GptInspection {
                    path: path.display().to_string(),
                    partition_index: Some(index),
                    has_gpt: true,
                    logical_block_size: Some(gpt.logical_block_size),
                    disk_guid: Some(gpt.disk_guid),
                    primary_header: gpt.primary_header,
                    backup_header: gpt.backup_header,
                    validation: gpt.validation,
                    partitions: gpt.partitions,
                    errors,
                });
            }
            Err(partition_errors) => {
                for error in partition_errors {
                    errors.push(format!("partition {index}: {error}"));
                }
            }
        }
    }

    if errors.is_empty() {
        errors.push("no GPT metadata found in DMG partitions".to_string());
    }

    Ok(GptInspection {
        path: path.display().to_string(),
        partition_index,
        has_gpt: false,
        logical_block_size: None,
        disk_guid: None,
        primary_header: None,
        backup_header: None,
        validation: GptValidation::default(),
        partitions: Vec::new(),
        errors,
    })
}

fn list_partitions_impl(path: &Path) -> Result<Vec<PartitionInfo>> {
    let parsed = parse_dmg(path)?;
    Ok(parsed.partitions.iter().map(to_partition_info).collect())
}

#[cfg(feature = "python")]
fn ensure_partition(
    partitions: &[PartitionRecord],
    index: usize,
) -> std::result::Result<&PartitionRecord, PyErr> {
    partitions.get(index).ok_or_else(|| {
        PyIndexError::new_err(format!(
            "partition index {index} out of range; available partitions: {}",
            partitions.len()
        ))
    })
}

fn normalize_relative_path(path: &Path) -> Result<String> {
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => bail!("unsafe path component in FAT32 entry: {}", path.display()),
        }
    }

    Ok(path
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/"))
}

fn is_special_fat_entry(name: &str) -> bool {
    name.is_empty() || name == "." || name == ".."
}

#[derive(Default)]
struct FilesystemBudget {
    entries: usize,
    bytes: u64,
}

impl FilesystemBudget {
    fn visit(&mut self, depth: usize) -> Result<()> {
        if depth > MAX_FILESYSTEM_DEPTH {
            bail!("filesystem nesting exceeds depth limit {MAX_FILESYSTEM_DEPTH}");
        }
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| anyhow!("filesystem entry count overflow"))?;
        if self.entries > MAX_FILESYSTEM_ENTRIES {
            bail!("filesystem entry count exceeds limit {MAX_FILESYSTEM_ENTRIES}");
        }
        Ok(())
    }

    fn add_file_bytes(&mut self, size: u64) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(size)
            .ok_or_else(|| anyhow!("filesystem byte count overflow"))?;
        if self.bytes > MAX_PARTITION_BYTES {
            bail!("filesystem output exceeds limit {MAX_PARTITION_BYTES}");
        }
        Ok(())
    }
}

fn list_fat32_entries_impl(path: &Path, partition_index: usize) -> Result<Vec<Fat32Entry>> {
    catch_dependency_panic("FAT32 directory listing", || {
        list_fat32_entries_unchecked(path, partition_index)
    })
}

fn list_fat32_entries_unchecked(path: &Path, partition_index: usize) -> Result<Vec<Fat32Entry>> {
    let parsed = parse_dmg(path)?;
    let partition = parsed
        .partitions
        .get(partition_index)
        .ok_or_else(|| anyhow!("partition index {partition_index} out of range"))?;
    let fs = open_fat_filesystem(path, partition)?;

    let mut entries = Vec::new();
    let mut budget = FilesystemBudget::default();
    collect_dir_entries(fs.root_dir(), PathBuf::new(), 0, &mut budget, &mut entries)?;
    Ok(entries)
}

fn collect_dir_entries<T: ReadWriteSeek>(
    dir: fatfs::Dir<'_, T>,
    prefix: PathBuf,
    depth: usize,
    budget: &mut FilesystemBudget,
    entries: &mut Vec<Fat32Entry>,
) -> Result<()> {
    for entry in dir.iter() {
        let entry = entry?;
        let name = entry.file_name();
        if is_special_fat_entry(name.as_str()) {
            continue;
        }
        budget.visit(depth)?;
        let rel_path = if prefix.as_os_str().is_empty() {
            PathBuf::from(name)
        } else {
            prefix.join(name)
        };
        let normalized = normalize_relative_path(&rel_path)?;
        let is_dir = entry.is_dir();
        let size = if entry.is_file() {
            Some(entry.len())
        } else {
            None
        };

        entries.push(Fat32Entry {
            path: normalized,
            is_dir,
            size,
        });

        if is_dir {
            collect_dir_entries(entry.to_dir(), rel_path, depth + 1, budget, entries)?;
        } else if let Some(size) = size {
            budget.add_file_bytes(size)?;
        }
    }
    Ok(())
}

fn ensure_safe_output_root(output_dir: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(output_dir) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!(
                    "output directory must not be a symlink: {}",
                    output_dir.display()
                );
            }
            if !metadata.is_dir() {
                bail!("output path is not a directory: {}", output_dir.display());
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(output_dir).with_context(|| {
                format!(
                    "unable to create output directory for FAT32 extraction: {}",
                    output_dir.display()
                )
            })?;
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "unable to inspect output directory: {}",
                    output_dir.display()
                )
            });
        }
    }

    let metadata = fs::symlink_metadata(output_dir).with_context(|| {
        format!(
            "unable to inspect output directory: {}",
            output_dir.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("unsafe output directory: {}", output_dir.display());
    }
    output_dir.canonicalize().with_context(|| {
        format!(
            "unable to resolve output directory: {}",
            output_dir.display()
        )
    })
}

fn ensure_safe_subdirectory(
    output_root: &Path,
    canonical_root: &Path,
    relative: &Path,
) -> Result<PathBuf> {
    let mut current = output_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            bail!("unsafe output path component in {}", relative.display());
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    bail!("refusing to traverse output symlink: {}", current.display());
                }
                if !metadata.is_dir() {
                    bail!(
                        "output path component is not a directory: {}",
                        current.display()
                    );
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).with_context(|| {
                    format!(
                        "unable to create extraction directory: {}",
                        current.display()
                    )
                })?;
                let metadata = fs::symlink_metadata(&current).with_context(|| {
                    format!(
                        "unable to inspect extraction directory: {}",
                        current.display()
                    )
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    bail!("unsafe extraction directory: {}", current.display());
                }
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "unable to inspect extraction directory: {}",
                        current.display()
                    )
                });
            }
        }

        let resolved = current.canonicalize().with_context(|| {
            format!(
                "unable to resolve extraction directory: {}",
                current.display()
            )
        })?;
        if !resolved.starts_with(canonical_root) {
            bail!(
                "extraction directory escapes output root: {}",
                current.display()
            );
        }
    }
    Ok(current)
}

fn extract_fat32_impl(
    path: &Path,
    output_dir: &Path,
    partition_index: usize,
    overwrite: bool,
) -> Result<Vec<String>> {
    catch_dependency_panic("FAT32 extraction", || {
        extract_fat32_unchecked(path, output_dir, partition_index, overwrite)
    })
}

fn extract_fat32_unchecked(
    path: &Path,
    output_dir: &Path,
    partition_index: usize,
    overwrite: bool,
) -> Result<Vec<String>> {
    let parsed = parse_dmg(path)?;
    let partition = parsed
        .partitions
        .get(partition_index)
        .ok_or_else(|| anyhow!("partition index {partition_index} out of range"))?;
    let fs = open_fat_filesystem(path, partition)?;

    let canonical_root = ensure_safe_output_root(output_dir)?;

    let mut context = FatExtractionContext {
        output_root: output_dir,
        canonical_root: &canonical_root,
        overwrite,
        budget: FilesystemBudget::default(),
        extracted: Vec::new(),
    };
    extract_dir_entries(fs.root_dir(), PathBuf::new(), 0, &mut context)?;

    Ok(context.extracted)
}

struct FatExtractionContext<'a> {
    output_root: &'a Path,
    canonical_root: &'a Path,
    overwrite: bool,
    budget: FilesystemBudget,
    extracted: Vec<String>,
}

fn extract_dir_entries<T: ReadWriteSeek>(
    dir: fatfs::Dir<'_, T>,
    prefix: PathBuf,
    depth: usize,
    context: &mut FatExtractionContext<'_>,
) -> Result<()> {
    for entry in dir.iter() {
        let entry = entry?;
        let name = entry.file_name();
        if is_special_fat_entry(name.as_str()) {
            continue;
        }
        context.budget.visit(depth)?;
        let rel_path = if prefix.as_os_str().is_empty() {
            PathBuf::from(name)
        } else {
            prefix.join(name)
        };
        let normalized = normalize_relative_path(&rel_path)?;

        if entry.is_dir() {
            ensure_safe_subdirectory(context.output_root, context.canonical_root, &rel_path)?;
            context.extracted.push(normalized.clone());
            extract_dir_entries(entry.to_dir(), rel_path, depth + 1, context)?;
            continue;
        }

        let parent_relative = rel_path.parent().unwrap_or_else(|| Path::new(""));
        let parent =
            ensure_safe_subdirectory(context.output_root, context.canonical_root, parent_relative)?;
        let destination = context.output_root.join(&rel_path);
        match fs::symlink_metadata(&destination) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    bail!(
                        "refusing to overwrite output symlink: {}",
                        destination.display()
                    );
                }
                if metadata.is_dir() {
                    bail!("output file path is a directory: {}", destination.display());
                }
                if !context.overwrite {
                    bail!(
                        "refusing to overwrite existing file {} (set overwrite=True)",
                        destination.display()
                    );
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("unable to inspect output file: {}", destination.display())
                });
            }
        }

        let file_size = entry.len();
        context.budget.add_file_bytes(file_size)?;
        let mut source = entry.to_file();
        let mut temporary = NamedTempFile::new_in(&parent).with_context(|| {
            format!(
                "unable to create temporary extraction file in {}",
                parent.display()
            )
        })?;
        let (written, limited_written) = {
            let mut limited = LimitedWriter::new(&mut temporary, file_size);
            let written = std::io::copy(&mut source, &mut limited).with_context(|| {
                format!("unable to write extracted file: {}", destination.display())
            })?;
            limited.flush()?;
            (written, limited.written())
        };
        if written != file_size || limited_written != file_size {
            bail!(
                "extracted file {} wrote {written} bytes; expected {file_size}",
                destination.display()
            );
        }
        record_sink_write(written);
        temporary.as_file().sync_all()?;
        if context.overwrite {
            temporary
                .persist(&destination)
                .map_err(|error| error.error)
                .with_context(|| {
                    format!("unable to atomically replace {}", destination.display())
                })?;
        } else {
            temporary
                .persist_noclobber(&destination)
                .map_err(|error| error.error)
                .with_context(|| {
                    format!("unable to atomically create {}", destination.display())
                })?;
        }

        context.extracted.push(normalized);
    }

    Ok(())
}

/// Internal entry points used by the `cargo-fuzz` targets.
///
/// These functions intentionally discard parser errors: fuzzing is looking for
/// panics, aborts, sanitizer findings, hangs, and uncontrolled resource use.
#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub mod fuzzing {
    use super::*;

    const CHUNK_TYPES: [u32; 10] = [
        ChunkType::Zero as u32,
        ChunkType::Raw as u32,
        ChunkType::Ignore as u32,
        ChunkType::Comment as u32,
        ChunkType::Adc as u32,
        ChunkType::Zlib as u32,
        ChunkType::Bzlib as u32,
        ChunkType::Lzfse as u32,
        ChunkType::Term as u32,
        0xdead_beef,
    ];

    pub fn dmg_parse(data: &[u8]) {
        let mut reader = Cursor::new(data);
        let _ = parse_dmg_reader(&mut reader, data.len() as u64);
    }

    pub fn blkx(data: &[u8]) {
        let _ = decode_blkx_table(data);
    }

    pub fn gpt(data: &[u8]) {
        let _ = parse_gpt_from_bytes(data);
    }

    pub fn chunk(data: &[u8]) {
        const HEADER_LEN: usize = 25;
        const MAX_ZERO_SECTORS: u64 = 4096;

        if data.len() < HEADER_LEN {
            return;
        }

        let selector = usize::from(data[0]) % CHUNK_TYPES.len();
        let sector_count = read_u64(&data[1..9]) % (MAX_ZERO_SECTORS + 1);
        let offset_seed = read_u64(&data[9..17]);
        let length_seed = read_u64(&data[17..25]);
        let payload = &data[HEADER_LEN..];

        let offset = usize::try_from(offset_seed % (payload.len() as u64 + 1)).unwrap_or(0);
        let available = payload.len().saturating_sub(offset);
        let length = usize::try_from(length_seed % (available as u64 + 1)).unwrap_or(0);

        let chunk = BlkxChunk {
            r#type: CHUNK_TYPES[selector],
            comment: 0,
            sector_number: 0,
            sector_count,
            compressed_offset: offset as u64,
            compressed_length: length as u64,
        };
        let _ = decode_chunk(
            &mut Cursor::new(payload),
            payload.len() as u64,
            &chunk,
            MAX_PARTITION_BYTES,
        );
    }

    pub fn image(path: &Path) {
        if let Ok(partitions) = list_partitions_impl(path) {
            // Exercise recursive FAT directory parsing without letting a single
            // image with thousands of partitions monopolize the campaign.
            for partition in partitions.iter().take(16) {
                let _ = list_fat32_entries_impl(path, partition.index);
            }
        }

        // Metadata inspection does not traverse Apple filesystem directories.
        // Listing the root and reading a small number of files reaches catalog,
        // inode, extent, and file-content paths driven by untrusted metadata.
        if let Ok(entries) = list_apple_entries_impl(path, "/") {
            for entry in entries.iter().filter(|entry| !entry.is_dir).take(2) {
                let _ = read_apple_file_impl(path, &entry.path);
            }
        }

        let _ = inspect_gpt_impl(path, None);
        let _ = inspect_filesystems_impl(path);
        let _ = inspect_impl(path);
    }

    fn read_u64(bytes: &[u8]) -> u64 {
        let mut array = [0_u8; 8];
        array.copy_from_slice(bytes);
        u64::from_le_bytes(array)
    }
}

#[cfg(feature = "python")]
fn to_py_runtime_error(error: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

#[cfg(all(feature = "python", feature = "benchmarking"))]
#[pyfunction]
fn _benchmark_metrics_reset() {
    reset_benchmark_metrics();
}

#[cfg(all(feature = "python", feature = "benchmarking"))]
#[pyfunction]
fn _benchmark_metrics_json() -> PyResult<String> {
    serde_json::to_string(&benchmark_metrics())
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))
}

#[cfg(feature = "python")]
#[pyfunction]
fn inspect_json(path: &str) -> PyResult<String> {
    let info = inspect_impl(Path::new(path)).map_err(to_py_runtime_error)?;
    serde_json::to_string(&info).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[cfg(feature = "python")]
#[pyfunction]
fn inspect_filesystems_json(path: &str) -> PyResult<String> {
    let info = inspect_filesystems_impl(Path::new(path)).map_err(to_py_runtime_error)?;
    serde_json::to_string(&info).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[cfg(feature = "python")]
#[pyfunction]
fn list_partitions_json(path: &str) -> PyResult<String> {
    let info = list_partitions_impl(Path::new(path)).map_err(to_py_runtime_error)?;
    serde_json::to_string(&info).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(signature = (path, partition_index=None, strict=false))]
fn inspect_gpt_json(path: &str, partition_index: Option<usize>, strict: bool) -> PyResult<String> {
    let mut inspection = inspect_gpt_impl(Path::new(path), partition_index).map_err(|error| {
        let message = error.to_string();
        if message.contains("out of range") {
            PyIndexError::new_err(message)
        } else {
            PyRuntimeError::new_err(message)
        }
    })?;

    if !strict && !inspection.has_gpt {
        // Probe mode treats "no GPT found" as an expected negative result.
        inspection.errors.clear();
    }

    if strict && !inspection.has_gpt {
        let joined = inspection.errors.join(" | ");
        return Err(PyRuntimeError::new_err(format!(
            "GPT inspection failed: {joined}"
        )));
    }

    serde_json::to_string(&inspection).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[cfg(feature = "python")]
#[pyfunction]
fn read_partition(py: Python<'_>, path: &str, index: usize) -> PyResult<Py<PyBytes>> {
    let parsed = parse_dmg(Path::new(path)).map_err(to_py_runtime_error)?;
    let partition = ensure_partition(&parsed.partitions, index)?;
    let output_size = partition_output_size(partition).map_err(to_py_runtime_error)?;
    let output_len =
        usize::try_from(output_size).map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    let data = PyBytes::new_with(py, output_len, |buffer| {
        let mut writer = Cursor::new(buffer);
        write_partition_to(Path::new(path), partition, &mut writer)
            .map(|_| ())
            .map_err(to_py_runtime_error)
    })?;
    Ok(data.unbind())
}

#[cfg(feature = "python")]
#[pyfunction]
fn extract_partition(path: &str, index: usize, output_path: &str) -> PyResult<u64> {
    let parsed = parse_dmg(Path::new(path)).map_err(to_py_runtime_error)?;
    let partition = ensure_partition(&parsed.partitions, index)?;

    let output = Path::new(output_path);
    let mut temporary = create_atomic_output(output).map_err(to_py_runtime_error)?;
    let written = write_partition_to(Path::new(path), partition, &mut temporary)
        .map_err(to_py_runtime_error)?;
    record_sink_write(written);
    persist_atomic_output(temporary, output).map_err(to_py_runtime_error)?;

    Ok(written)
}

#[cfg(feature = "python")]
#[pyfunction]
fn compute_data_checksum(path: &str) -> PyResult<u32> {
    let parsed = parse_dmg(Path::new(path)).map_err(to_py_runtime_error)?;
    data_fork_checksum(Path::new(path), &parsed.koly).map_err(to_py_runtime_error)
}

#[cfg(feature = "python")]
#[pyfunction]
fn verify_data_checksum(path: &str) -> PyResult<bool> {
    let parsed = parse_dmg(Path::new(path)).map_err(to_py_runtime_error)?;
    let declared = u32::from(parsed.koly.data_fork_digest);
    let computed =
        data_fork_checksum(Path::new(path), &parsed.koly).map_err(to_py_runtime_error)?;
    Ok(declared == computed)
}

#[cfg(feature = "python")]
#[pyfunction]
fn list_fat32_entries_json(path: &str, partition_index: usize) -> PyResult<String> {
    let entries = list_fat32_entries_impl(Path::new(path), partition_index).map_err(|error| {
        let message = error.to_string();
        if message.contains("out of range") {
            PyIndexError::new_err(message)
        } else {
            PyRuntimeError::new_err(message)
        }
    })?;

    serde_json::to_string(&entries).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(signature = (path, output_dir, partition_index=1, overwrite=false))]
fn extract_fat32_json(
    path: &str,
    output_dir: &str,
    partition_index: usize,
    overwrite: bool,
) -> PyResult<String> {
    let extracted = extract_fat32_impl(
        Path::new(path),
        Path::new(output_dir),
        partition_index,
        overwrite,
    )
    .map_err(|error| {
        let message = error.to_string();
        if message.contains("out of range") {
            PyIndexError::new_err(message)
        } else {
            PyRuntimeError::new_err(message)
        }
    })?;

    serde_json::to_string(&extracted).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(signature = (path, directory_path="/"))]
fn list_apple_entries_json(path: &str, directory_path: &str) -> PyResult<String> {
    let entries =
        list_apple_entries_impl(Path::new(path), directory_path).map_err(to_py_runtime_error)?;
    serde_json::to_string(&entries).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[cfg(feature = "python")]
#[pyfunction]
fn read_apple_file(py: Python<'_>, path: &str, file_path: &str) -> PyResult<Py<PyBytes>> {
    let data = read_apple_file_impl(Path::new(path), file_path).map_err(to_py_runtime_error)?;
    Ok(PyBytes::new(py, &data).unbind())
}

#[cfg(feature = "python")]
#[pyfunction]
fn extract_apple_file(path: &str, file_path: &str, output_path: &str) -> PyResult<u64> {
    extract_apple_file_impl(Path::new(path), file_path, Path::new(output_path))
        .map_err(to_py_runtime_error)
}

#[cfg(feature = "python")]
#[pyfunction]
#[pyo3(signature = (source_dir, output_path, volume_label="PYDMG", total_sectors=32768))]
fn create_dmg(
    source_dir: &str,
    output_path: &str,
    volume_label: &str,
    total_sectors: u32,
) -> PyResult<()> {
    let source = Path::new(source_dir);
    let output = Path::new(output_path);

    if total_sectors == 0 {
        return Err(PyValueError::new_err(
            "total_sectors must be greater than zero",
        ));
    }

    let output_bytes = u64::from(total_sectors)
        .checked_mul(SECTOR_SIZE)
        .ok_or_else(|| PyValueError::new_err("requested DMG size overflows u64"))?;
    if output_bytes > MAX_PARTITION_BYTES {
        return Err(PyValueError::new_err(format!(
            "requested DMG size {output_bytes} exceeds safety limit {MAX_PARTITION_BYTES}"
        )));
    }

    if !source.exists() {
        return Err(PyValueError::new_err(format!(
            "source directory does not exist: {}",
            source.display()
        )));
    }

    if !source.is_dir() {
        return Err(PyValueError::new_err(format!(
            "source path is not a directory: {}",
            source.display()
        )));
    }

    let temporary = create_atomic_output(output).map_err(to_py_runtime_error)?;
    create_dmg_with_dependency(source, temporary.path(), volume_label, total_sectors)
        .map_err(to_py_runtime_error)?;
    persist_atomic_output(temporary, output).map_err(to_py_runtime_error)
}

#[cfg(feature = "python")]
#[pymodule]
fn _pydmg(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(inspect_json, module)?)?;
    module.add_function(wrap_pyfunction!(inspect_filesystems_json, module)?)?;
    module.add_function(wrap_pyfunction!(list_partitions_json, module)?)?;
    module.add_function(wrap_pyfunction!(inspect_gpt_json, module)?)?;
    module.add_function(wrap_pyfunction!(read_partition, module)?)?;
    module.add_function(wrap_pyfunction!(extract_partition, module)?)?;
    module.add_function(wrap_pyfunction!(compute_data_checksum, module)?)?;
    module.add_function(wrap_pyfunction!(verify_data_checksum, module)?)?;
    module.add_function(wrap_pyfunction!(list_fat32_entries_json, module)?)?;
    module.add_function(wrap_pyfunction!(extract_fat32_json, module)?)?;
    module.add_function(wrap_pyfunction!(list_apple_entries_json, module)?)?;
    module.add_function(wrap_pyfunction!(read_apple_file, module)?)?;
    module.add_function(wrap_pyfunction!(extract_apple_file, module)?)?;
    module.add_function(wrap_pyfunction!(create_dmg, module)?)?;
    #[cfg(feature = "benchmarking")]
    {
        module.add_function(wrap_pyfunction!(_benchmark_metrics_reset, module)?)?;
        module.add_function(wrap_pyfunction!(_benchmark_metrics_json, module)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bzip2::{write::BzEncoder, Compression as BzCompression};
    use flate2::{write::ZlibEncoder, Compression as ZlibCompression};

    fn compressed_chunk(kind: ChunkType, compressed_length: usize) -> BlkxChunk {
        BlkxChunk {
            r#type: kind as u32,
            comment: 0,
            sector_number: 0,
            sector_count: 1,
            compressed_offset: 0,
            compressed_length: compressed_length as u64,
        }
    }

    fn encode_blkx(table: &BlkxTable) -> Vec<u8> {
        let mut encoded = Vec::new();
        table.write_to(&mut encoded).expect("encode blkx table");
        encoded
    }

    #[test]
    fn blkx_rejects_unbounded_chunk_count_before_parsing() {
        let mut table = vec![0_u8; BLKX_HEADER_SIZE];
        table[..4].copy_from_slice(b"mish");
        table[BLKX_CHUNK_COUNT_OFFSET..BLKX_HEADER_SIZE].copy_from_slice(&u32::MAX.to_be_bytes());

        let error = decode_blkx_table(&table).expect_err("oversized count must be rejected");
        assert!(error.to_string().contains("chunk count"));
    }

    #[test]
    fn blkx_rejects_truncated_chunk_array() {
        let mut table = vec![0_u8; BLKX_HEADER_SIZE];
        table[..4].copy_from_slice(b"mish");
        table[BLKX_CHUNK_COUNT_OFFSET..BLKX_HEADER_SIZE].copy_from_slice(&1_u32.to_be_bytes());

        let error = decode_blkx_table(&table).expect_err("truncated table must be rejected");
        assert!(error.to_string().contains("requiring"));
    }

    #[test]
    fn blkx_rejects_gaps_and_large_dependency_allocations() {
        let mut gapped = BlkxTable {
            sector_count: 2,
            ..BlkxTable::default()
        };
        gapped.chunks = vec![
            BlkxChunk::new(ChunkType::Raw, 1, 1, 0, SECTOR_SIZE),
            BlkxChunk::term(2, SECTOR_SIZE),
        ];
        let error = decode_blkx_table(&encode_blkx(&gapped))
            .expect_err("non-contiguous output must be rejected");
        assert!(error.to_string().contains("starts at sector"));

        let oversized_sectors = MAX_EXPANDED_CHUNK_BYTES / SECTOR_SIZE + 1;
        let mut oversized = BlkxTable {
            sector_count: oversized_sectors,
            ..BlkxTable::default()
        };
        oversized.chunks = vec![
            BlkxChunk::new(ChunkType::Zero, 0, oversized_sectors, 0, 0),
            BlkxChunk::term(oversized_sectors, 0),
        ];
        let error = decode_blkx_table(&encode_blkx(&oversized))
            .expect_err("oversized dependency allocation must be rejected");
        assert!(error.to_string().contains("expanded size"));
    }

    #[test]
    fn filesystem_preflight_rejects_attacker_controlled_block_sizes() {
        let mut hfs = vec![0_u8; HFS_VOLUME_HEADER_OFFSET as usize + 48];
        let header = HFS_VOLUME_HEADER_OFFSET as usize;
        hfs[header..header + 2].copy_from_slice(b"H+");
        hfs[header + 40..header + 44].copy_from_slice(&u32::MAX.to_be_bytes());
        let error = validate_hfs_reader(&mut Cursor::new(hfs))
            .expect_err("oversized HFS+ allocation block must be rejected");
        assert!(error.to_string().contains("allocation block size"));

        let mut apfs = vec![0_u8; APFS_SUPERBLOCK_PREFIX_SIZE];
        apfs[32..36].copy_from_slice(b"NXSB");
        apfs[36..40].copy_from_slice(&u32::MAX.to_le_bytes());
        let error = validate_apfs_reader(&mut Cursor::new(apfs))
            .expect_err("oversized APFS block must be rejected");
        assert!(error.to_string().contains("APFS block size"));
    }

    #[test]
    fn file_ranges_reject_overflow_and_eof() {
        assert!(validate_file_range(10, 11, 0, "test").is_err());
        assert!(validate_file_range(10, 9, 2, "test").is_err());
        assert!(validate_file_range(u64::MAX, u64::MAX, 1, "test").is_err());
        assert!(validate_file_range(10, 10, 0, "test").is_ok());
    }

    #[test]
    fn compressed_decoders_round_trip_with_bounds() {
        let payload = vec![b'p'; SECTOR_SIZE as usize];

        let mut zlib_encoder = ZlibEncoder::new(Vec::new(), ZlibCompression::default());
        zlib_encoder.write_all(&payload).expect("encode zlib input");
        let zlib = zlib_encoder.finish().expect("finish zlib stream");
        let zlib_chunk = compressed_chunk(ChunkType::Zlib, zlib.len());
        assert_eq!(
            decode_chunk(
                &mut Cursor::new(&zlib),
                zlib.len() as u64,
                &zlib_chunk,
                SECTOR_SIZE,
            )
            .expect("decode zlib stream"),
            payload
        );

        let mut bzip_encoder = BzEncoder::new(Vec::new(), BzCompression::default());
        bzip_encoder
            .write_all(&payload)
            .expect("encode bzip2 input");
        let bzip = bzip_encoder.finish().expect("finish bzip2 stream");
        let bzip_chunk = compressed_chunk(ChunkType::Bzlib, bzip.len());
        assert_eq!(
            decode_chunk(
                &mut Cursor::new(&bzip),
                bzip.len() as u64,
                &bzip_chunk,
                SECTOR_SIZE,
            )
            .expect("decode bzip2 stream"),
            payload
        );

        let mut lzfse_buffer = vec![0_u8; payload.len() * 2];
        let lzfse_len =
            lzfse::encode_buffer(&payload, &mut lzfse_buffer).expect("encode LZFSE input");
        lzfse_buffer.truncate(lzfse_len);
        let lzfse_chunk = compressed_chunk(ChunkType::Lzfse, lzfse_buffer.len());
        assert_eq!(
            decode_chunk(
                &mut Cursor::new(&lzfse_buffer),
                lzfse_buffer.len() as u64,
                &lzfse_chunk,
                SECTOR_SIZE,
            )
            .expect("decode LZFSE stream"),
            payload
        );
    }

    #[test]
    fn compressed_decoder_rejects_expansion_past_declared_sectors() {
        let payload = vec![0_u8; (SECTOR_SIZE * 2) as usize];
        let mut encoder = ZlibEncoder::new(Vec::new(), ZlibCompression::default());
        encoder.write_all(&payload).expect("encode zlib input");
        let compressed = encoder.finish().expect("finish zlib stream");
        let chunk = compressed_chunk(ChunkType::Zlib, compressed.len());

        let error = decode_chunk(
            &mut Cursor::new(&compressed),
            compressed.len() as u64,
            &chunk,
            SECTOR_SIZE,
        )
        .expect_err("oversized expansion must be rejected");
        assert!(error.to_string().contains("exceeds limit"));
    }

    #[test]
    fn partition_reader_decodes_only_the_requested_chunk() {
        let first = vec![b'a'; SECTOR_SIZE as usize];
        let second = vec![b'b'; SECTOR_SIZE as usize];
        let mut first_encoder = ZlibEncoder::new(Vec::new(), ZlibCompression::default());
        first_encoder.write_all(&first).expect("encode first chunk");
        let first_compressed = first_encoder.finish().expect("finish first chunk");
        let mut second_encoder = ZlibEncoder::new(Vec::new(), ZlibCompression::default());
        second_encoder
            .write_all(&second)
            .expect("encode second chunk");
        let second_compressed = second_encoder.finish().expect("finish second chunk");

        let mut image = NamedTempFile::new().expect("create image");
        image
            .write_all(&first_compressed)
            .expect("write first chunk");
        image
            .write_all(&second_compressed)
            .expect("write second chunk");
        image.flush().expect("flush image");

        let partition = PartitionRecord {
            index: 0,
            id: "0".to_string(),
            name: "test".to_string(),
            cfname: None,
            attributes: None,
            table: BlkxTable {
                sector_count: 2,
                chunks: vec![
                    BlkxChunk::new(ChunkType::Zlib, 0, 1, 0, first_compressed.len() as u64),
                    BlkxChunk::new(
                        ChunkType::Zlib,
                        1,
                        1,
                        first_compressed.len() as u64,
                        second_compressed.len() as u64,
                    ),
                    BlkxChunk::term(2, (first_compressed.len() + second_compressed.len()) as u64),
                ],
                ..BlkxTable::default()
            },
        };

        let mut reader = PartitionReader::open(image.path(), &partition).expect("open partition");
        let mut prefix = [0_u8; 8];
        reader.read_exact(&mut prefix).expect("read first span");
        assert_eq!(prefix, [b'a'; 8]);
        assert_eq!(reader.cached_span, Some(0));

        reader
            .seek(SeekFrom::Start(SECTOR_SIZE))
            .expect("seek to second span");
        reader.read_exact(&mut prefix).expect("read second span");
        assert_eq!(prefix, [b'b'; 8]);
        assert_eq!(reader.cached_span, Some(1));
        assert!(reader.cache.capacity() < (SECTOR_SIZE * 2) as usize);
    }

    #[test]
    fn partition_writer_rejects_short_declared_output() {
        let mut image = NamedTempFile::new().expect("create image");
        image.write_all(&[7]).expect("write raw byte");
        image.flush().expect("flush image");
        let partition = PartitionRecord {
            index: 0,
            id: "0".to_string(),
            name: "short".to_string(),
            cfname: None,
            attributes: None,
            table: BlkxTable {
                sector_count: 1,
                chunks: vec![
                    BlkxChunk::new(ChunkType::Raw, 0, 1, 0, 1),
                    BlkxChunk::term(1, 1),
                ],
                ..BlkxTable::default()
            },
        };

        let error = write_partition_to(image.path(), &partition, &mut Vec::new())
            .expect_err("short expanded output must be rejected");
        assert!(error.to_string().contains("produced 1 bytes; expected 512"));
    }

    #[test]
    fn deeply_nested_plist_is_rejected() {
        let mut value = Value::String("leaf".to_string());
        for _ in 0..=MAX_PLIST_DEPTH {
            value = Value::Array(vec![value]);
        }
        assert!(validate_plist_structure(&value).is_err());
    }

    #[test]
    fn deeply_nested_plist_is_rejected_while_streaming() {
        let mut xml = String::from("<plist version=\"1.0\">");
        for _ in 0..=MAX_PLIST_DEPTH {
            xml.push_str("<array>");
        }
        xml.push_str("<string>leaf</string>");
        for _ in 0..=MAX_PLIST_DEPTH {
            xml.push_str("</array>");
        }
        xml.push_str("</plist>");

        let error = parse_plist_payload(xml.as_bytes())
            .expect_err("streaming parser must enforce the nesting limit");
        assert!(error.to_string().contains("nesting-depth limit"));
    }

    #[test]
    fn binary_plist_collection_allocation_is_preflighted() {
        let mut bytes = b"bplist00".to_vec();
        bytes.push(0xaf); // Array with an extended length.
        bytes.push(0x12); // Four-byte unsigned length.
        bytes.extend_from_slice(&(MAX_PLIST_NODES as u32 + 1).to_be_bytes());
        let offset_table_offset = bytes.len() as u64;
        bytes.push(8); // The single object starts immediately after the magic.

        let mut trailer = [0_u8; 32];
        trailer[6] = 1; // One-byte object offsets.
        trailer[7] = 1; // One-byte object references.
        trailer[8..16].copy_from_slice(&1_u64.to_be_bytes());
        trailer[16..24].copy_from_slice(&0_u64.to_be_bytes());
        trailer[24..32].copy_from_slice(&offset_table_offset.to_be_bytes());
        bytes.extend_from_slice(&trailer);

        let error = preflight_binary_plist(&bytes)
            .expect_err("oversized binary collection must be rejected before allocation");
        assert!(error.to_string().contains("collection length"));
        assert!(error.to_string().contains("node limit"));
    }

    #[test]
    fn dependency_panics_become_regular_errors() {
        let error = catch_dependency_panic::<()>("test dependency", || {
            panic!("synthetic dependency panic")
        })
        .expect_err("panic must be contained");
        let message = error.to_string();
        assert!(message.contains("test dependency"));
        assert!(message.contains("upstream parser panic"));
    }

    #[test]
    fn filesystem_io_budget_rejects_repeated_reads_and_excess_bytes() {
        let mut operation_limited = BudgetedIo::with_limits(Cursor::new(vec![1_u8, 2, 3]), 16, 1);
        let mut byte = [0_u8; 1];
        operation_limited
            .read_exact(&mut byte)
            .expect("first read is within the operation budget");
        let error = operation_limited
            .read_exact(&mut byte)
            .expect_err("second read must exceed the operation budget");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("operation budget"));

        let mut byte_limited = BudgetedIo::with_limits(Cursor::new(vec![1_u8, 2, 3]), 2, 16);
        let error = byte_limited
            .read_exact(&mut [0_u8; 3])
            .expect_err("read beyond the byte budget must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("byte budget"));
    }

    #[cfg(unix)]
    #[test]
    fn dmg_creation_contains_path_dependency_panic() {
        let destination = tempfile::tempdir().expect("create destination directory");
        let error = create_dmg_with_dependency(
            Path::new("/"),
            &destination.path().join("output.dmg"),
            "TEST",
            32_768,
        )
        .expect_err("dependency panic must be contained");
        let message = error.to_string();
        assert!(message.contains("DMG creation"));
        assert!(message.contains("upstream parser panic"));
    }

    #[test]
    fn filesystem_partition_limits_declared_and_written_bytes() {
        assert!(validate_apple_partition_size(MAX_PARTITION_BYTES).is_ok());
        assert!(validate_apple_partition_size(MAX_PARTITION_BYTES + 1).is_err());

        let mut output = Vec::new();
        let error = LimitedWriter::new(&mut output, 3)
            .write_all(&[1, 2, 3, 4])
            .expect_err("writer must reject output beyond its limit");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(output.is_empty());
    }

    #[test]
    fn parses_a_valid_gpt_image() {
        const TOTAL_BYTES: usize = 1024 * 70;
        let mut disk = GptConfig::new()
            .writable(true)
            .logical_block_size(LogicalBlockSize::Lb512)
            .create_from_device(Cursor::new(vec![0_u8; TOTAL_BYTES]), None)
            .expect("create in-memory GPT");
        disk.add_partition("test", 4096, gpt::partition_types::BASIC, 0, None)
            .expect("add GPT partition");
        let device = disk.write().expect("write GPT headers");
        let bytes = device.into_inner();

        let parsed = parse_gpt_from_bytes(&bytes).expect("parse valid GPT");
        assert_eq!(parsed.logical_block_size, SECTOR_SIZE);
        assert_eq!(parsed.partitions.len(), 1);
        assert_eq!(parsed.partitions[0].name, "test");
        assert!(parsed.validation.primary_header_valid);
    }

    #[test]
    fn gpt_partition_entry_size_assertion_is_preflighted() {
        const TOTAL_BYTES: usize = 1024 * 70;
        const PRIMARY_HEADER_OFFSET: usize = 512;
        const HEADER_LENGTH: usize = 92;
        const HEADER_CRC_OFFSET: usize = 16;
        const PARTITION_SIZE_OFFSET: usize = 84;

        let disk = GptConfig::new()
            .writable(true)
            .logical_block_size(LogicalBlockSize::Lb512)
            .create_from_device(Cursor::new(vec![0_u8; TOTAL_BYTES]), None)
            .expect("create in-memory GPT");
        let device = disk.write().expect("write GPT headers");
        let mut bytes = device.into_inner();

        let header = &mut bytes[PRIMARY_HEADER_OFFSET..PRIMARY_HEADER_OFFSET + HEADER_LENGTH];
        header[PARTITION_SIZE_OFFSET..PARTITION_SIZE_OFFSET + 4]
            .copy_from_slice(&129_u32.to_le_bytes());
        header[HEADER_CRC_OFFSET..HEADER_CRC_OFFSET + 4].fill(0);
        let crc32 = crc32fast::hash(header);
        header[HEADER_CRC_OFFSET..HEADER_CRC_OFFSET + 4].copy_from_slice(&crc32.to_le_bytes());

        let contained = catch_unwind(AssertUnwindSafe(|| parse_gpt_from_bytes(&bytes)))
            .expect("crafted GPT must not panic");
        let errors = contained.expect_err("unsupported entry size must be rejected");
        assert!(errors.iter().any(|error| error.contains("entry size 129")));
    }

    #[test]
    fn gpt_partition_table_crc_mismatch_is_rejected() {
        const TOTAL_BYTES: usize = 1024 * 70;
        const PRIMARY_TABLE_OFFSET: usize = 2 * 512;

        let disk = GptConfig::new()
            .writable(true)
            .logical_block_size(LogicalBlockSize::Lb512)
            .create_from_device(Cursor::new(vec![0_u8; TOTAL_BYTES]), None)
            .expect("create in-memory GPT");
        let device = disk.write().expect("write GPT headers");
        let mut bytes = device.into_inner();
        bytes[PRIMARY_TABLE_OFFSET] ^= 0xff;

        let errors = parse_gpt_from_bytes(&bytes)
            .expect_err("partition table CRC mismatch must be rejected");
        assert!(errors
            .iter()
            .any(|error| error.contains("partition table CRC32 mismatch")));
    }
}
