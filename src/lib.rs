#![allow(clippy::useless_conversion)]

use std::{
    fs::{self, File},
    io::{BufReader, Cursor, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use apple_dmg::{BlkxChunk, BlkxTable, ChunkType, KolyTrailer};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use crc32fast::Hasher;
use dpp::{DmgPipeline, FilesystemHandle};
use fatfs::{FatType, FileSystem, FsOptions, ReadWriteSeek};
use flate2::read::ZlibDecoder;
use gpt::{disk::LogicalBlockSize, GptConfig};
use plist::{Dictionary, Value};
use pyo3::{
    exceptions::{PyIndexError, PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyBytes, PyModule},
};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};

const SECTOR_SIZE: u64 = 512;

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
    Dmg(FilesystemHandle),
    HfsRaw(dpp::hfsplus::HfsVolume<BufReader<File>>),
    ApfsRaw(dpp::apfs::ApfsVolume<BufReader<File>>),
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

    let koly = KolyTrailer::read_from(&mut reader)
        .with_context(|| format!("unable to read koly trailer from {}", path.display()))?;

    let plist_end = koly
        .plist_offset
        .checked_add(koly.plist_length)
        .ok_or_else(|| anyhow!("plist offset overflow"))?;
    if plist_end > file_size {
        bail!(
            "plist range {}..{} exceeds file size {}",
            koly.plist_offset,
            plist_end,
            file_size
        );
    }

    reader
        .seek(SeekFrom::Start(koly.plist_offset))
        .context("unable to seek to plist")?;
    let plist_len =
        usize::try_from(koly.plist_length).context("plist length does not fit usize")?;
    let mut plist_bytes = vec![0_u8; plist_len];
    reader
        .read_exact(&mut plist_bytes)
        .context("unable to read plist payload")?;

    let plist = Value::from_reader(Cursor::new(&plist_bytes))
        .or_else(|_| Value::from_reader_xml(Cursor::new(&plist_bytes)))
        .context("unable to parse plist payload")?;

    let partitions = extract_partitions(&plist)?;

    Ok(ParsedDmg {
        file_size,
        koly,
        plist,
        partitions,
    })
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

    let mut partitions = Vec::with_capacity(blkx_entries.len());

    for (source_index, entry) in blkx_entries.iter().enumerate() {
        let dict = expect_dict(entry, "partition entry")?;
        let Some(data_value) = dict.get("Data") else {
            continue;
        };
        let table_bytes = match data_value {
            Value::Data(bytes) => bytes.clone(),
            _ => continue,
        };

        let table = BlkxTable::read_from(&mut Cursor::new(&table_bytes))
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

fn read_exact_at<R: Read + Seek>(reader: &mut R, offset: u64, length: u64) -> Result<Vec<u8>> {
    let len = usize::try_from(length).context("payload length does not fit usize")?;
    reader
        .seek(SeekFrom::Start(offset))
        .with_context(|| format!("unable to seek to payload at offset {offset}"))?;
    let mut buffer = vec![0_u8; len];
    reader
        .read_exact(&mut buffer)
        .with_context(|| format!("unable to read payload at offset {offset}, length {length}"))?;
    Ok(buffer)
}

fn decode_chunk<R: Read + Seek>(reader: &mut R, chunk: &BlkxChunk) -> Result<Vec<u8>> {
    let chunk_type = chunk
        .ty()
        .ok_or_else(|| anyhow!("unknown chunk type: 0x{:08x}", chunk.r#type))?;

    match chunk_type {
        ChunkType::Raw => read_exact_at(reader, chunk.compressed_offset, chunk.compressed_length),
        ChunkType::Zlib => {
            let compressed =
                read_exact_at(reader, chunk.compressed_offset, chunk.compressed_length)?;
            let mut decoder = ZlibDecoder::new(&compressed[..]);
            let mut decoded = Vec::new();
            decoder
                .read_to_end(&mut decoded)
                .context("unable to zlib-decompress chunk")?;
            Ok(decoded)
        }
        ChunkType::Zero | ChunkType::Ignore | ChunkType::Comment => {
            let raw_len = if chunk.sector_count > 0 {
                chunk
                    .sector_count
                    .checked_mul(SECTOR_SIZE)
                    .ok_or_else(|| anyhow!("chunk sector size overflow"))?
            } else {
                chunk.compressed_length
            };
            let len = usize::try_from(raw_len).context("zero-fill length does not fit usize")?;
            Ok(vec![0_u8; len])
        }
        ChunkType::Term => Ok(Vec::new()),
        ChunkType::Adc | ChunkType::Bzlib | ChunkType::Lzfse => {
            bail!("unsupported compression type {chunk_type:?}")
        }
    }
}

fn read_partition_bytes(path: &Path, partition: &PartitionRecord) -> Result<Vec<u8>> {
    let mut reader = BufReader::new(
        File::open(path).with_context(|| format!("unable to open DMG: {}", path.display()))?,
    );

    let mut out = Vec::new();
    for chunk in &partition.table.chunks {
        let mut chunk_data = decode_chunk(&mut reader, chunk)?;
        out.append(&mut chunk_data);
    }

    Ok(out)
}

fn data_fork_checksum(path: &Path, koly: &KolyTrailer) -> Result<u32> {
    let mut reader = BufReader::new(
        File::open(path).with_context(|| format!("unable to open DMG: {}", path.display()))?,
    );

    reader
        .seek(SeekFrom::Start(koly.data_fork_offset))
        .context("unable to seek to data fork")?;

    let mut hasher = Hasher::new();
    let mut limited = (&mut reader).take(koly.data_fork_length);
    let mut buffer = [0_u8; 128 * 1024];

    loop {
        let read = limited
            .read(&mut buffer)
            .context("unable to read data fork")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hasher.finalize())
}

fn value_to_short_string(value: &Value) -> String {
    match value {
        Value::Boolean(v) => v.to_string(),
        Value::Data(bytes) => format!("<{} bytes>", bytes.len()),
        Value::Date(date) => format!("{date:?}"),
        Value::Real(real) => real.to_string(),
        Value::Integer(int) => format!("{int:?}"),
        Value::String(s) => s.clone(),
        Value::Array(values) => {
            let parts: Vec<String> = values.iter().map(value_to_short_string).collect();
            format!("[{}]", parts.join(", "))
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

fn push_candidate(target: &mut Vec<MetadataCandidate>, path: &[String], key: &str, value: &Value) {
    let rendered = value_to_short_string(value);
    target.push(MetadataCandidate {
        path: path.join("."),
        key: key.to_string(),
        value: rendered,
    });
}

fn collect_metadata_candidates(value: &Value) -> MetadataCandidates {
    fn walk(value: &Value, path: &mut Vec<String>, out: &mut MetadataCandidates) {
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
                        push_candidate(&mut out.creation_dates, path, key, child);
                    }

                    if key_lower.contains("author")
                        || key_lower.contains("creator")
                        || key_lower.contains("owner")
                        || key_lower.contains("publisher")
                    {
                        push_candidate(&mut out.authors, path, key, child);
                    }

                    if key_lower.contains("application")
                        || key_lower.contains("app")
                        || key_lower.contains("software")
                        || key_lower.contains("program")
                        || key_lower.contains("generator")
                        || key_lower.contains("creator")
                    {
                        push_candidate(&mut out.creation_applications, path, key, child);
                    }

                    walk(child, path, out);
                    let _ = path.pop();
                }
            }
            Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    path.push(index.to_string());
                    walk(child, path, out);
                    let _ = path.pop();
                }
            }
            _ => {}
        }
    }

    let mut out = MetadataCandidates::default();
    let mut path = Vec::new();
    walk(value, &mut path, &mut out);
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

fn parse_gpt_from_bytes(bytes: &[u8]) -> std::result::Result<ParsedGpt, Vec<String>> {
    let mut errors = Vec::new();
    let block_sizes = [LogicalBlockSize::Lb512, LogicalBlockSize::Lb4096];

    for block_size in block_sizes {
        let lb_bytes = logical_block_size_bytes(block_size);
        let device = Cursor::new(bytes.to_vec());
        let config = GptConfig::new()
            .writable(false)
            .logical_block_size(block_size)
            .only_valid_headers(false);

        match config.open_from_device(device) {
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

fn detect_fat_filesystems(
    path: &Path,
    partitions: &[PartitionRecord],
) -> (Vec<FatFilesystemMetadata>, Vec<String>) {
    let mut filesystems = Vec::new();
    let mut errors = Vec::new();

    for partition in partitions {
        let bytes = match read_partition_bytes(path, partition) {
            Ok(bytes) => bytes,
            Err(error) => {
                errors.push(format!(
                    "partition {} ({}) read failed: {}",
                    partition.index, partition.name, error
                ));
                continue;
            }
        };

        let fs = match FileSystem::new(Cursor::new(bytes), FsOptions::new()) {
            Ok(fs) => fs,
            Err(_) => continue,
        };

        let stats = fs.stats().ok();
        let root_volume_label = fs.read_volume_label_from_root_dir().ok().flatten();

        filesystems.push(FatFilesystemMetadata {
            partition_index: partition.index,
            partition_name: partition.name.clone(),
            fat_type: fat_type_name(fs.fat_type()).to_string(),
            volume_id: fs.volume_id(),
            volume_label: fs.volume_label(),
            root_volume_label,
            cluster_size: stats.map(|s| s.cluster_size()),
            total_clusters: stats.map(|s| s.total_clusters()),
            free_clusters: stats.map(|s| s.free_clusters()),
        });
    }

    (filesystems, errors)
}

fn fs_type_name(fs_type: dpp::FsType) -> &'static str {
    match fs_type {
        dpp::FsType::HfsPlus => "hfsplus",
        dpp::FsType::Apfs => "apfs",
    }
}

fn open_apple_filesystem(path: &Path) -> Result<AppleFsHandle> {
    let mut errors = Vec::new();

    match DmgPipeline::open(path) {
        Ok(mut pipeline) => match pipeline.open_filesystem() {
            Ok(filesystem) => return Ok(AppleFsHandle::Dmg(filesystem)),
            Err(error) => errors.push(format!("dmg filesystem detection failed: {error}")),
        },
        Err(error) => errors.push(format!("dmg pipeline open failed: {error}")),
    }

    match File::open(path) {
        Ok(file) => {
            let reader = BufReader::new(file);
            match dpp::hfsplus::HfsVolume::open(reader) {
                Ok(volume) => return Ok(AppleFsHandle::HfsRaw(volume)),
                Err(error) => errors.push(format!("raw HFS+ parse failed: {error}")),
            }
        }
        Err(error) => {
            errors.push(format!("unable to open image for raw HFS+ parse: {error}"));
        }
    }

    match File::open(path) {
        Ok(file) => {
            let reader = BufReader::new(file);
            match dpp::apfs::ApfsVolume::open(reader) {
                Ok(volume) => return Ok(AppleFsHandle::ApfsRaw(volume)),
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
        AppleFsHandle::Dmg(filesystem) => {
            let info = filesystem.volume_info();
            let mut metadata = AppleFilesystemMetadata {
                fs_type: fs_type_name(info.fs_type).to_string(),
                block_size: info.block_size,
                file_count: info.file_count,
                directory_count: info.directory_count,
                volume_name: info.name.clone(),
                symlink_count: info.symlink_count,
                total_blocks: info.total_blocks,
                free_blocks: info.free_blocks,
                version: info.version,
                is_hfsx: info.is_hfsx,
                volume_create_time: None,
                volume_modify_time: None,
                root_create_time: None,
                root_modify_time: None,
            };

            match filesystem {
                FilesystemHandle::Hfs(hfs) => {
                    let header = hfs.volume_header();
                    metadata.volume_create_time = Some(i64::from(header.create_date));
                    metadata.volume_modify_time = Some(i64::from(header.modify_date));
                }
                FilesystemHandle::Apfs(apfs) => {
                    if let Ok(root) = apfs.stat("/") {
                        metadata.root_create_time = Some(root.create_time);
                        metadata.root_modify_time = Some(root.modify_time);
                    }
                }
            }

            metadata
        }
        AppleFsHandle::HfsRaw(hfs) => {
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
        AppleFsHandle::ApfsRaw(apfs) => {
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
    let mut handle = open_apple_filesystem(path)?;
    Ok(apple_metadata_from_handle(&mut handle))
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
    let normalized_dir = normalize_apple_path(directory_path);
    let mut handle = open_apple_filesystem(path)?;

    match &mut handle {
        AppleFsHandle::Dmg(filesystem) => {
            let entries = filesystem
                .list_directory(&normalized_dir)
                .with_context(|| format!("unable to list directory: {normalized_dir}"))?;
            Ok(entries
                .into_iter()
                .map(|entry| AppleFsEntry {
                    path: join_apple_entry_path(&normalized_dir, &entry.name),
                    is_dir: matches!(entry.kind, dpp::FsEntryKind::Directory),
                    size: entry.size,
                })
                .collect::<Vec<_>>())
        }
        AppleFsHandle::HfsRaw(hfs) => {
            let entries = hfs
                .list_directory(&normalized_dir)
                .with_context(|| format!("unable to list directory: {normalized_dir}"))?;
            Ok(entries
                .into_iter()
                .map(|entry| AppleFsEntry {
                    path: join_apple_entry_path(&normalized_dir, &entry.name),
                    is_dir: matches!(entry.kind, dpp::hfsplus::EntryKind::Directory),
                    size: entry.size,
                })
                .collect::<Vec<_>>())
        }
        AppleFsHandle::ApfsRaw(apfs) => {
            let entries = apfs
                .list_directory(&normalized_dir)
                .with_context(|| format!("unable to list directory: {normalized_dir}"))?;
            Ok(entries
                .into_iter()
                .map(|entry| AppleFsEntry {
                    path: join_apple_entry_path(&normalized_dir, &entry.name),
                    is_dir: matches!(entry.kind, dpp::apfs::EntryKind::Directory),
                    size: entry.size,
                })
                .collect::<Vec<_>>())
        }
    }
}

fn read_apple_file_impl(path: &Path, file_path: &str) -> Result<Vec<u8>> {
    let normalized_path = normalize_apple_path(file_path);
    let mut handle = open_apple_filesystem(path)?;

    match &mut handle {
        AppleFsHandle::Dmg(filesystem) => filesystem
            .read_file(&normalized_path)
            .with_context(|| format!("unable to read filesystem file: {normalized_path}")),
        AppleFsHandle::HfsRaw(hfs) => hfs
            .read_file(&normalized_path)
            .with_context(|| format!("unable to read filesystem file: {normalized_path}")),
        AppleFsHandle::ApfsRaw(apfs) => apfs
            .read_file(&normalized_path)
            .with_context(|| format!("unable to read filesystem file: {normalized_path}")),
    }
}

fn extract_apple_file_impl(path: &Path, file_path: &str, output_path: &Path) -> Result<u64> {
    let normalized_path = normalize_apple_path(file_path);
    let mut handle = open_apple_filesystem(path)?;

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!("unable to create output directory: {}", parent.display())
            })?;
        }
    }

    let mut destination = File::create(output_path)
        .with_context(|| format!("unable to create output file: {}", output_path.display()))?;
    let written = match &mut handle {
        AppleFsHandle::Dmg(filesystem) => filesystem
            .read_file_to(&normalized_path, &mut destination)
            .with_context(|| {
                format!("unable to extract file from filesystem: {normalized_path}")
            })?,
        AppleFsHandle::HfsRaw(hfs) => hfs
            .read_file_to(&normalized_path, &mut destination)
            .with_context(|| {
                format!("unable to extract file from filesystem: {normalized_path}")
            })?,
        AppleFsHandle::ApfsRaw(apfs) => apfs
            .read_file_to(&normalized_path, &mut destination)
            .with_context(|| {
                format!("unable to extract file from filesystem: {normalized_path}")
            })?,
    };
    destination.flush()?;
    Ok(written)
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

fn list_fat32_entries_impl(path: &Path, partition_index: usize) -> Result<Vec<Fat32Entry>> {
    let parsed = parse_dmg(path)?;
    let partition = parsed
        .partitions
        .get(partition_index)
        .ok_or_else(|| anyhow!("partition index {partition_index} out of range"))?;
    let bytes = read_partition_bytes(path, partition)?;
    let fs = FileSystem::new(Cursor::new(bytes), FsOptions::new())
        .context("unable to open FAT32 filesystem from partition data")?;

    let mut entries = Vec::new();
    collect_dir_entries(fs.root_dir(), PathBuf::new(), &mut entries)?;
    Ok(entries)
}

fn collect_dir_entries<T: ReadWriteSeek>(
    dir: fatfs::Dir<'_, T>,
    prefix: PathBuf,
    entries: &mut Vec<Fat32Entry>,
) -> Result<()> {
    for entry in dir.iter() {
        let entry = entry?;
        let name = entry.file_name();
        if is_special_fat_entry(name.as_str()) {
            continue;
        }
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
            collect_dir_entries(entry.to_dir(), rel_path, entries)?;
        }
    }
    Ok(())
}

fn extract_fat32_impl(
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
    let bytes = read_partition_bytes(path, partition)?;
    let fs = FileSystem::new(Cursor::new(bytes), FsOptions::new())
        .context("unable to open FAT32 filesystem from partition data")?;

    fs::create_dir_all(output_dir).with_context(|| {
        format!(
            "unable to create output directory for FAT32 extraction: {}",
            output_dir.display()
        )
    })?;

    let mut extracted = Vec::new();
    extract_dir_entries(
        fs.root_dir(),
        output_dir,
        PathBuf::new(),
        overwrite,
        &mut extracted,
    )?;

    Ok(extracted)
}

fn extract_dir_entries<T: ReadWriteSeek>(
    dir: fatfs::Dir<'_, T>,
    output_root: &Path,
    prefix: PathBuf,
    overwrite: bool,
    extracted: &mut Vec<String>,
) -> Result<()> {
    for entry in dir.iter() {
        let entry = entry?;
        let name = entry.file_name();
        if is_special_fat_entry(name.as_str()) {
            continue;
        }
        let rel_path = if prefix.as_os_str().is_empty() {
            PathBuf::from(name)
        } else {
            prefix.join(name)
        };
        let normalized = normalize_relative_path(&rel_path)?;
        let destination = output_root.join(&rel_path);

        if entry.is_dir() {
            fs::create_dir_all(&destination).with_context(|| {
                format!(
                    "unable to create directory during extraction: {}",
                    destination.display()
                )
            })?;
            extracted.push(normalized.clone());
            extract_dir_entries(entry.to_dir(), output_root, rel_path, overwrite, extracted)?;
            continue;
        }

        if destination.exists() && !overwrite {
            bail!(
                "refusing to overwrite existing file {} (set overwrite=True)",
                destination.display()
            );
        }

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("unable to create parent directory: {}", parent.display())
            })?;
        }

        let mut source = entry.to_file();
        let mut destination_file = File::create(&destination).with_context(|| {
            format!("unable to create extracted file: {}", destination.display())
        })?;
        std::io::copy(&mut source, &mut destination_file).with_context(|| {
            format!("unable to write extracted file: {}", destination.display())
        })?;
        destination_file.flush()?;

        extracted.push(normalized);
    }

    Ok(())
}

fn to_py_runtime_error(error: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

#[pyfunction]
fn inspect_json(path: &str) -> PyResult<String> {
    let info = inspect_impl(Path::new(path)).map_err(to_py_runtime_error)?;
    serde_json::to_string(&info).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[pyfunction]
fn inspect_filesystems_json(path: &str) -> PyResult<String> {
    let info = inspect_filesystems_impl(Path::new(path)).map_err(to_py_runtime_error)?;
    serde_json::to_string(&info).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[pyfunction]
fn list_partitions_json(path: &str) -> PyResult<String> {
    let info = list_partitions_impl(Path::new(path)).map_err(to_py_runtime_error)?;
    serde_json::to_string(&info).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

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

#[pyfunction]
fn read_partition(py: Python<'_>, path: &str, index: usize) -> PyResult<Py<PyBytes>> {
    let parsed = parse_dmg(Path::new(path)).map_err(to_py_runtime_error)?;
    let partition = ensure_partition(&parsed.partitions, index)?;
    let data = read_partition_bytes(Path::new(path), partition).map_err(to_py_runtime_error)?;
    Ok(PyBytes::new_bound(py, &data).into())
}

#[pyfunction]
fn extract_partition(path: &str, index: usize, output_path: &str) -> PyResult<u64> {
    let parsed = parse_dmg(Path::new(path)).map_err(to_py_runtime_error)?;
    let partition = ensure_partition(&parsed.partitions, index)?;
    let data = read_partition_bytes(Path::new(path), partition).map_err(to_py_runtime_error)?;

    let output = Path::new(output_path);
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| PyRuntimeError::new_err(err.to_string()))?;
        }
    }

    let mut writer =
        File::create(output).map_err(|err| PyRuntimeError::new_err(err.to_string()))?;
    writer
        .write_all(&data)
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))?;
    writer
        .flush()
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))?;

    u64::try_from(data.len()).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[pyfunction]
fn compute_data_checksum(path: &str) -> PyResult<u32> {
    let parsed = parse_dmg(Path::new(path)).map_err(to_py_runtime_error)?;
    data_fork_checksum(Path::new(path), &parsed.koly).map_err(to_py_runtime_error)
}

#[pyfunction]
fn verify_data_checksum(path: &str) -> PyResult<bool> {
    let parsed = parse_dmg(Path::new(path)).map_err(to_py_runtime_error)?;
    let declared = u32::from(parsed.koly.data_fork_digest);
    let computed =
        data_fork_checksum(Path::new(path), &parsed.koly).map_err(to_py_runtime_error)?;
    Ok(declared == computed)
}

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

#[pyfunction]
#[pyo3(signature = (path, directory_path="/"))]
fn list_apple_entries_json(path: &str, directory_path: &str) -> PyResult<String> {
    let entries =
        list_apple_entries_impl(Path::new(path), directory_path).map_err(to_py_runtime_error)?;
    serde_json::to_string(&entries).map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[pyfunction]
fn read_apple_file(py: Python<'_>, path: &str, file_path: &str) -> PyResult<Py<PyBytes>> {
    let data = read_apple_file_impl(Path::new(path), file_path).map_err(to_py_runtime_error)?;
    Ok(PyBytes::new_bound(py, &data).into())
}

#[pyfunction]
fn extract_apple_file(path: &str, file_path: &str, output_path: &str) -> PyResult<u64> {
    extract_apple_file_impl(Path::new(path), file_path, Path::new(output_path))
        .map_err(to_py_runtime_error)
}

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

    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| PyRuntimeError::new_err(err.to_string()))?;
        }
    }

    apple_dmg::create_dmg(source, output, volume_label, total_sectors).map_err(to_py_runtime_error)
}

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
    Ok(())
}
