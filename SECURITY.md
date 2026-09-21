# Security Policy

`pydmg` parses disk-image and filesystem structures that may be attacker-controlled. Treat every
DMG, raw HFS+ image, raw APFS image, plist, partition table, filename, and compressed chunk as
untrusted input.

## Supported versions

`pydmg` is currently alpha software. Security fixes are applied to the latest release and the
default branch; older releases may not receive backports.

| Version | Security support |
| --- | --- |
| Latest `0.1.x` release | Best effort |
| Default branch | Active development |
| Older releases | Not supported |

## Assessed threat model

The active assessment assumes an attacker can choose every byte of an image before it is handed to
`pydmg`, but cannot change that image, a DMG-creation source tree, or an output tree while an
operation is running. The caller chooses output files/directories and they are not writable by a
hostile concurrent process.

The deployment workflow in scope reads image-provided paths, metadata, and file data, and may
write file data to one caller-selected destination. It does not replay an image's path,
permissions, ownership, timestamps, or other metadata onto that destination. Therefore
cross-process check/use races and embedded-name extraction attacks are not active threats in this
profile.

CPU and memory exhaustion are deferred for this assessment. The workflow does not require data
provenance or authenticity, and it records selected content only as inert binary data or text; it
does not execute that content or pass it to an active interpreter. Text remains data and must be
escaped if a future caller introduces an active rendering or command sink.

`extract_fat32()` is a public bulk-extraction API that does create an image-derived directory tree.
It is outside that narrower deployment workflow; its existing path normalization, symlink, count,
depth, and atomic-write protections remain defense in depth. `create_dmg()` likewise assumes its
caller-controlled source tree is immutable for the operation.

## Reporting a vulnerability

Prefer a private GitHub security advisory through the repository's **Security** tab. Include:

- the affected API and platform;
- the smallest reproducer or malformed image available;
- observed impact, including memory, CPU, or filesystem effects;
- whether the issue crosses a trust or privilege boundary; and
- any suggested mitigation.

Do not attach a sensitive proof of concept to a public issue. If private reporting is unavailable,
open a minimal public issue asking the maintainer to establish a private contact channel.

## Current security posture

The dated assessment, open findings, evidence, and remediation status are maintained in
[`AUDIT.md`](AUDIT.md). The fuzzing design and reproducible commands are in
[`FUZZING.md`](FUZZING.md).

Repository-owned demonstrated issues have local guards and regressions. The vulnerable unused XAR
dependency path has been removed and no advisory exceptions remain. Plist budgets now run while
events are streamed, dangerous binary-plist collection lengths are rejected before allocation,
GPT headers are preflighted before the dependency assertion, and FAT reads have byte/operation
budgets plus panic containment. FAT access now decompresses only requested bounded BLKX chunks;
complete partition reads stream to their final sink and must match the declared expanded size. No
demonstrated repository-owned vulnerability remains active
under the assessed threat model. The APFS parser panic remains external, HFS/APFS B-tree traversal
budgets remain upstream gaps, and fixed in-process ceilings cannot bound all third-party parser
CPU work; their residual CPU/memory availability impact is deferred in this assessment.

The following remain defense-in-depth guidance for broader deployments:

- parse untrusted images in a sandboxed, unprivileged process with memory, CPU, file-size, and
  wall-clock limits;
- if using the separately scoped bulk FAT extractor, use a caller-controlled output directory and
  do not run it as a privileged account;
- treat CRC32 results as accidental-corruption checks, not authenticity or signature validation;
- independently validate output paths before consuming extracted files; and
- assume `RuntimeError` can represent a contained upstream parser panic and retain stderr when
  investigating a malformed image.

## Enforced in-process limits

These are safety ceilings, not format maxima or a substitute for host isolation:

| Resource | Limit |
| --- | ---: |
| Plist payload | 64 MiB |
| Plist event nesting / nodes | 128 / 1,000,000 |
| BLKX chunks | 262,144 |
| GPT entries / entry size | 16,384 / exactly 128 B |
| Expanded partition or aggregate FAT output | 512 MiB |
| FAT/HFS+/APFS dependency reads | 576 MiB / 2,000,000 operations per opened filesystem |
| Compressed chunk input | 256 MiB |
| Expanded UDIF chunk | 64 MiB |
| APFS block / checkpoint descriptors | 4..64 KiB / 4,096 |
| HFS+ allocation block | 512 B..64 MiB, power of two |
| One HFS+/APFS file | 512 MiB |
| Filesystem entries / nesting | 100,000 / 128 |
| Created DMG | 512 MiB |

All trailer and chunk ranges use checked arithmetic and must fit the real input. BLKX output must
be contiguous and cover its declared partition. Zlib, bzip2, and LZFSE output is bounded. ADC
remains unsupported upstream and is rejected before output.

FAT extraction rejects symlink roots/components/final paths and writes temporary sibling files
before atomic persistence. Direct partition, Apple-file, and DMG outputs also use atomic
replacement, so a failure preserves the prior destination and a final symlink is replaced rather
than followed. Concurrent mutation is excluded by the assessed threat model.

## Security boundaries and non-goals

- `verify_data_checksum()` compares a DMG-declared CRC32 with a computed CRC32. CRC32 is not
  collision-resistant and does not establish origin, authenticity, or freedom from tampering;
  authenticity is not required by the assessed workflow.
- `pydmg` does not validate Apple code signatures, notarization, package signatures, or developer
  identity; this is an accepted non-goal for inert data recording.
- Filesystem metadata and names returned by an image are untrusted strings.
- A successful parse means the implemented structural checks passed; it does not mean the image is
  safe to mount, execute, or install.
- Fuzzing reduces risk but is not proof that parsing arbitrary input is safe.
- `apfs 0.2.4` can still panic internally on malformed B-tree data. `pydmg` catches unwind at its
  HFS+/APFS entry points, but `catch_unwind` does not catch aborting panics, allocation aborts,
  or stack overflow. APFS and HFS B-tree traversals also lack upstream visited-node budgets; the
  shared dependency-read ceiling stops I/O-driven loops but cannot enforce semantic recursion
  depth or bound CPU spent between reads. The residual CPU/memory availability impact is deferred
  under the assessed threat model.
- `udif 0.3.4` materializes compressed and expanded blocks internally. Local BLKX layout and size
  checks bound the reachable allocations. Residual memory consumption is deferred in this
  assessment; a process memory limit remains broader defense in depth.
- `fatfs 0.3.6` does not detect cyclic cluster chains. Repository-owned read-operation and byte
  budgets stop such a static input with an ordinary error, and the FAT entry points contain
  unwinding dependency panics. Any remaining CPU impact is deferred in this assessment.
- `gpt 4.1.0` asserts that partition entries are 128 bytes. Repository preflight rejects any valid
  selected header with a different entry size, caps its count/range, validates the partition-table
  CRC32, avoids a full input clone, and contains any other unwinding dependency panic.
- Do not publish raw fuzz artifacts or unsanitized failure logs from public CI. Crash inputs can
  disclose an unpatched vulnerability; reproduce and report them through a private channel with
  owner-only storage.
- The locked graph contains only patched `quick-xml 0.41.0`; the prior `dpp -> xara -> quick-xml
  0.37.5` path and both RustSec exceptions were removed because `pydmg` does not expose XAR/PKG
  parsing. `security/audit-exceptions.json` is intentionally empty.

## Maintainer security checks

Run these checks before a release and after dependency changes:

```bash
pytest -q
ruff check .
mypy
bandit -q -r python scripts
cargo fmt --all -- --check
cargo fmt --manifest-path fuzz/Cargo.toml -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo clippy --manifest-path fuzz/Cargo.toml --bins -- -D warnings
cargo test --locked --all-targets --all-features
cargo check --manifest-path fuzz/Cargo.toml --bins
cargo audit
python -m pip_audit .
python -m pip_audit
python scripts/check_audit_exceptions.py
python scripts/check_licenses.py
python scripts/build_pydoc.py --cleanup
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
python scripts/verify_versions.py
```

Run the bounded fuzz smoke campaigns described in [`FUZZING.md`](FUZZING.md), and run longer
campaigns for parser or dependency changes. The release workflow repeats these gates. A release is
not considered security-clean while a relevant unexplained advisory, sanitizer finding, new
reproducible crash, hang, uncontrolled allocation, or output-directory escape remains unresolved.
Known external exceptions require explicit review and must stay visible in [`AUDIT.md`](AUDIT.md).
