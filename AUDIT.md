# Code Quality And Security Audit

Assessment date: **2026-07-18**
Remediation review: **2026-07-19**
Performance/toolchain review: **2026-09-21**
Code baseline: `de11c42b3950` on `master`, plus the remediation change set documented here
Scope: Rust core, Python API, tests, fixtures, packaging, dependencies, documentation,
CI/release workflows, fuzzing, and hostile-input behavior

This is a point-in-time engineering assessment, not a certification. Findings remain in this file
after remediation so the original evidence and residual risk are reviewable. Status terms mean:

- **Resolved locally**: repository code and a deterministic regression address the demonstrated
  path; final status still depends on normal review and CI.
- **Contained locally; upstream open**: the repository converts or bounds the failure, but the
  dependency defect still exists and must not be edited outside this repository.
- **External open**: no repository-only change can remove the root cause; an upstream release or
  external tool/state change is required.
- **Out of scope under assessed threat model**: historically relevant behavior that requires a
  concurrent or different-capability attacker excluded by the deployment model below.
- **Deferred under assessed threat model**: a retained finding whose only relevant impact in this
  review is CPU or memory exhaustion, which the current assessment explicitly excludes.

## Assessed threat model

An attacker may construct or modify every byte of an input image before delivery. Once processing
starts, the image is immutable, and the attacker cannot concurrently mutate the input, a
DMG-creation source tree, or any output tree. The caller controls output locations.

The scoped deployment reads image-provided names, metadata, and file data, and may write only file
data to a single caller-selected destination. It does not apply image-provided paths, permissions,
ownership, timestamps, or other metadata to that file. The public `extract_fat32()` bulk API is a
separately assessed exception because it creates image-derived paths; it is not used by the
narrower deployment workflow, and its existing path defenses remain in place.

For this assessment, CPU and memory exhaustion are deferred rather than treated as active security
impacts. The workflow also does not require provenance or authenticity: extracted bytes and text
are recorded as inert data and are not executed or passed to an active interpreter. CRC32 and
structural parsing remain correctly documented as non-authenticating operations.

## Executive summary

The repository-owned lint, type, security-lint, test, documentation, range-validation,
decompression, output-safety, CI, release, and fuzz-infrastructure findings have been remediated.
The current local suite passes 22 Python integration/adversarial tests and 20 native Rust tests
(19 cross-platform plus one Unix path-panic regression).
Ruff, strict mypy, Bandit, rustfmt, and Clippy with warnings denied are green. Python line coverage
remains 90% and now has a gate; final local and hosted Rust 1.98.1 runs both record 80.03% native
lines against the separate 70% workflow gate.

The 171 GB BLKX allocation is rejected before entering `apple-dmg`. The malformed APFS case is
caught at this repository's dependency boundary and returned as `RuntimeError`, but `apfs 0.2.4`
still contains two confirmed B-tree panic paths and unbounded traversal/count paths. APFS/HFS block
sizes, checkpoint scans, and UDIF block allocations are now preflighted locally. The unused
`dpp -> xara` path that retained vulnerable `quick-xml 0.37.5` was removed; the locked graph now
contains only patched `quick-xml 0.41.0`, and no RustSec exceptions remain. ADC-compressed chunks
remain unsupported upstream but fail closed; bzip2 and LZFSE are bounded and tested locally.
Additional static-input review found and fixed three repository boundary gaps: `gpt 4.1.0`'s
partition-entry assertion is now preflighted and panic-contained, cyclic FAT work is stopped by
read budgets, and plist node/depth/collection limits are enforced before value materialization.

No demonstrated repository-owned vulnerability remains active under the assessed threat model.
Host CPU/memory limits and the fixed in-process budgets remain useful defense in depth for broader
deployments, but residual third-party resource behavior is deferred in this review.

| Area | Current result |
| --- | --- |
| Python tests | 22 passed |
| Native Rust tests | 20 passed on macOS; 19 cross-platform |
| Python coverage | 90%; enforced at 90% in CI/release |
| Native Rust coverage | Final local and hosted Rust 1.98.1 result 80.03% lines; 70% gate |
| Python lint/type | Expanded Ruff rules, strict mypy, and consumer stub check pass |
| Security lint | Bandit passes; three exact false positives have inline rationale |
| Rust lint | rustfmt and Clippy `-D warnings` pass |
| Fuzz harness | All five local ASan campaigns pass; hosted smoke/scheduled gates retained |
| Python dependency audit | No runtime dependencies or known vulnerability in the last completed run |
| Rust dependency audit | PyO3 and quick-xml advisories resolved; no configured exceptions remain |
| Documentation | README, API docstrings, security policy, audit, fuzz guide, and `AGENTS.md` updated |
| Performance | Dated nine-sample before/after evidence; clean-SHA CI/release artifact gates |
| Release readiness | Version 0.1.3 is published from the verified tag; Rust 1.98.1/maturin are pinned and MSRV remains tested at 1.88 |

## Security findings

### SEC-001 — Attacker-controlled BLKX count requested a 171 GB allocation

- Original severity: **High**
- Status: **Resolved locally; deterministic regressions pass**
- CWE: CWE-789, uncontrolled memory allocation
- Regression: Rust tests `blkx_rejects_unbounded_chunk_count_before_parsing` and
  `blkx_rejects_truncated_chunk_array`

The original `blkx` libFuzzer campaign stopped after 26 executions when a 240-byte input requested
approximately 171.8 GB. A 210-byte minimized input still requested 171.1 GB; its SHA-256 is
`ef078bfac7421766dff20a406d47b9c24d5b0ec2228ad0f361e243bcfb7ff41a`. The artifact remains outside
the repository.

The allocation occurs in `apple-dmg` before it validates that all declared chunks exist. The local
`decode_blkx_table` boundary now checks the signature, minimum header, count conversion, a 262,144
chunk ceiling, exact `204 + count * 40` byte availability, contiguous output layout, terminator
placement, per-chunk and aggregate sizes, and arithmetic overflow before calling that parser. The
fuzz target uses this production boundary rather than calling the dependency directly.

### SEC-002 — Malformed APFS B-tree data panics in `apfs 0.2.4`

- Original severity: **High**
- Status: **Contained locally; upstream availability work deferred under assessed threat model**
- CWE: CWE-129/CWE-248
- Regressions: `dependency_panics_become_regular_errors`, APFS block-size preflight, and the exact
  artifact retained privately

A one-byte mutation of the valid APFS fixture triggers:

```text
apfs-0.2.4/src/btree.rs:177:44
range start index 5413 out of range for slice of length 4096
```

The artifact SHA-256 is
`dff3f0ad754f4340083f708adebd8bba000cae9bb9ebab3c1020c394a456cd81`. It is not committed.

A separate synthetic B-tree node reaches another panic at `apfs-0.2.4/src/btree.rs:253` when
`val_area_end - val_off` underflows. Source review also found attacker-derived
`Vec::with_capacity(btn_nkeys)`, unchecked checkpoint iteration, and recursive B-tree descent
without a depth or visited-node budget. These are availability risks: a bounds panic was
demonstrated, while allocator abort, stack overflow, and nontermination remain credible from the
code paths. No unsafe-memory access, memory corruption, information disclosure, or code execution
was demonstrated.

All local HFS+/APFS metadata, listing, and read/extract entry points use `catch_unwind` and turn an
unwinding dependency panic into an ordinary error. APFS input is also preflighted to the documented
4 KiB..64 KiB power-of-two block range, a 4,096-block checkpoint-scan ceiling, and ranges within the
container. The exact mutation produces `RuntimeError` through the rebuilt Python extension.

`catch_unwind` cannot catch aborting panics, allocator aborts, stack overflows, or hangs. The actual
B-tree validation and traversal budgets therefore must be fixed in `apfs` and a fixed release
adopted; registry source was not modified. The whole-image harness replaces libFuzzer's aborting
panic hook so the production catch boundary is exercised. The exact artifact now completes, and a
post-preflight 68-second whole-image ASan campaign completed without a new finding, so scheduled
whole-image fuzzing is a required gate rather than `continue-on-error`.

### SEC-003 — Expanded-image and recursive operations lacked resource budgets

- Original severity: **High**
- Status: **Mitigated locally; residual CPU/allocation risk deferred under assessed threat model**
- CWE: CWE-409/CWE-770
- Regressions: `compressed_decoder_rejects_expansion_past_declared_sectors`,
  `blkx_rejects_gaps_and_large_dependency_allocations`, `deeply_nested_plist_is_rejected`,
  `filesystem_preflight_rejects_attacker_controlled_block_sizes`, and the integration suite

The implementation now enforces fixed checked ceilings before local allocations:

| Resource | Limit |
| --- | ---: |
| Plist payload | 64 MiB |
| Plist event nesting / nodes | 128 / 1,000,000 |
| BLKX chunks or plist BLKX entries | 262,144 |
| GPT entries / entry size | 16,384 / exactly 128 B |
| Expanded partition / aggregate FAT output | 512 MiB |
| FAT/HFS+/APFS dependency reads | 576 MiB / 2,000,000 operations per opened filesystem |
| One compressed chunk | 256 MiB |
| One expanded UDIF chunk | 64 MiB |
| APFS block / checkpoint descriptors | 4..64 KiB / 4,096 |
| HFS+ allocation block | 512 B..64 MiB, power of two |
| One HFS+/APFS file | 512 MiB |
| Filesystem entries / nesting | 100,000 / 128 |
| Metadata candidates / rendered value | 10,000 / 4,096 characters |
| Created DMG | 512 MiB |

Zlib and bzip2 decode through a one-byte-over-limit reader directly into the destination; LZFSE
reuses bounded compressed and declared-size buffers; zero/raw chunks and aggregate partition output
are checked before allocation. Complete decodes must produce exactly the declared expanded size.
FAT operations use a read-only seekable BLKX device with a one-chunk cache and decompress only the
spans requested by `fatfs`, while the existing dependency read-byte and operation budgets remain
outside that device. Binary plist
object counts and collection reference lengths are preflighted, and all plist encodings pass
through a bounded event stream before a `Value` is built. BLKX data
chunks must be contiguous, cover the declared partition exactly, and expand to no more than 64 MiB
each. This prevents `udif` from allocating attacker-sized output gaps and caps its LZFSE temporary
buffer at 128 MiB. Its whole-data-fork checksum allocation was replaced at the call boundary by the
repository's fixed-buffer streaming CRC32 check. FAT traversal has depth, count, and output-byte
budgets. FAT, HFS+, and APFS readers share dependency-read-byte and dependency-read-operation
budgets, while Apple file output also uses a limited writer.

Residual risk remains because `udif`, `hfsplus`, and `apfs` still do internal work before a local
output writer sees it. HFS/APFS B-tree walks have no upstream visited-node or semantic depth
budget. The I/O ceiling terminates read-driven loops but cannot catch an APFS stack overflow or
serve as a complete CPU deadline. These facts remain recorded for broader deployments, but their
CPU/memory availability impact is explicitly deferred in the current assessment. Host sandbox and
process limits remain defense-in-depth recommendations rather than current-scope blockers.

### SEC-004 — FAT extraction followed pre-existing symlinks outside the root

- Original severity: **Medium** (High for privileged extraction into an attacker-writable tree)
- Status: **Demonstrated path resolved; concurrent mutation out of scope**
- CWE: CWE-59
- Regressions: `test_fat32_extraction_rejects_dangling_file_symlink` and
  `test_fat32_extraction_rejects_symlink_output_root`

The reproduced dangling-link escape is rejected. The extractor rejects a symlink output root,
walks and checks every parent component, verifies canonical parents remain beneath the root,
rejects final symlink destinations even when dangling, writes a sibling temporary file, and uses
atomic persist/persist-no-clobber behavior.

The separately scoped bulk API still uses image-derived names, so these checks remain useful. A
cross-process check/use race would require an attacker able to mutate the output tree during the
operation; that capability is explicitly excluded by the assessed threat model.

### SEC-005 — Checksum verification accepted a data-fork range beyond EOF

- Original severity: **Medium**
- Status: **Resolved locally**
- CWE: CWE-1284
- Regression: `test_checksum_rejects_data_fork_range_beyond_eof` and
  `file_ranges_reject_overflow_and_eof`

Every relevant `koly` range is checked with overflow-safe addition against file size during parse.
Checksum reading must consume exactly the declared length. The prior beyond-EOF/zero-CRC case now
fails in `inspect`, `compute_data_checksum`, and `verify_data_checksum`.

### SEC-006 — Two transitive Rust dependency vulnerabilities entered through unused XAR support

- Original severity: **Medium** overall
- Status: **Resolved locally; vulnerable dependency path removed and exceptions deleted**

PyO3 was upgraded from 0.22.6 to 0.29.0, removing RUSTSEC-2025-0020 and RUSTSEC-2026-0177 from the
resolved graph. The historical quick-xml path was:

```text
quick-xml 0.37.5 -> xara 0.2.2 -> dpp 0.3.5 -> pydmg
```

The latest available `dpp 0.4.2 -> xara 0.3.2` graph was also checked and still declared
`quick-xml = "0.37"`. `pydmg` did not expose the XAR/PKG functionality that caused that dependency,
so the broad `dpp` pipeline was replaced with direct `udif 0.3.4`, `hfsplus 0.2.4`, and
`apfs 0.2.4` dependencies plus a locally bounded DMG-filesystem adapter. The adapter validates the
DMG through the production boundary and limits the temporary expanded partition to 512 MiB before
opening the filesystem parser.

| Advisory | Historical root cause | Current handling |
| --- | --- | --- |
| RUSTSEC-2026-0194 | Quadratic attributes in `quick-xml 0.37.5` | Vulnerable crate removed; locked graph uses 0.41.0 |
| RUSTSEC-2026-0195 | Namespace-reader memory exhaustion in `quick-xml 0.37.5` | Vulnerable crate removed; locked graph uses 0.41.0 |

`cargo tree --locked -i quick-xml` now reports only `quick-xml 0.41.0` through `plist`.
`.cargo/audit.toml` has no ignores, `security/audit-exceptions.json` is empty, and the exception
validator enforces that state. CI installs pinned `cargo-audit 0.22.2` and fails on any advisory.

### SEC-007 — Release dependency resolution was not reproducibly locked

- Original severity: **Medium**
- Status: **Resolved locally**

The root `Cargo.lock` is now included and release/CI commands use `--locked`. Only
`/fuzz/Cargo.lock` remains ignored. Dependency changes must update and review the root lockfile.

### SEC-008 — Direct-output APIs followed links and left partial output on failure

- Original severity: **Low hardening issue**
- Status: **Resolved locally for caller-selected final paths**
- Regressions: `test_partition_extract_replaces_link_without_following_it`,
  `test_create_dmg_replaces_link_without_following_it`, and preservation assertion in
  `test_apple_filesystem_helpers_raise_when_absent`

Partition extraction, Apple-file extraction, and DMG creation now write a sibling temporary file,
flush/synchronize it, and atomically replace the final path only after success. Existing symlinks
are replaced as directory entries rather than followed; their targets remain unchanged. An error
leaves the prior destination intact.

### SEC-009 — CRC32 and structural parsing do not establish authenticity

- Severity: **Informational**
- Status: **Documented; accepted non-goal under assessed threat model**

CRC32 detects accidental corruption, not malicious tampering or publisher identity. `pydmg` does
not verify Apple code signatures, notarization, package signatures, or developer identity. The
assessed workflow does not require provenance: it records selected content as inert bytes or text
and performs no active execution.

### SEC-010 — Filesystem auto-detection bypassed the expanded-partition budget

- Original severity: **Medium**
- Status: **Resolved locally**
- CWE: CWE-770
- Regression: `filesystem_partition_limits_declared_and_written_bytes` plus the real HFS+/APFS
  integration fixtures

The former `dpp::DmgPipeline::open_filesystem()` path streamed a selected filesystem partition to a
temporary file before any repository-owned `LimitedWriter` was involved. Direct Apple-filesystem
APIs could therefore bypass the intended 512 MiB expanded-partition ceiling and consume excessive
temporary storage even though the separate partition-reading API was bounded.

The replacement `udif` adapter first runs the complete local DMG/BLKX validation boundary, rejects
a selected partition whose declared size exceeds 512 MiB, and streams through `LimitedWriter` so
the same ceiling applies to actual bytes written even if dependency metadata is inconsistent. It
also checks the dependency's reported byte count against the writer's observed count, validates
contiguous chunk layout and per-chunk allocation size, and performs CRC32 verification with a
fixed-buffer local stream instead of `udif`'s whole-data-fork allocation before opening HFS+ or
APFS.

### SEC-011 — Public fuzz artifacts could disclose an unpatched crash input

- Original severity: **Medium process vulnerability**
- Status: **Resolved locally for GitHub Actions; existing external files require operator care**
- CWE: CWE-200

The repository is public, and the initial workflows uploaded raw `cargo-fuzz` crash directories on
failure. Users with repository read access can download workflow artifacts, so a previously unknown
security input could have been public before private triage. Retention limits reduce duration but do
not make a public artifact confidential. See GitHub's
[artifact access documentation](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/download-workflow-artifacts).

Fuzz and release workflows now capture libFuzzer output in a runner-local file, print only final
statistics on success and a sanitized failure notice on error, and never upload crash artifacts.
Sensitive inputs must be reproduced and preserved only on a private runner or local host with
owner-only directory permissions. This avoids both artifact upload and libFuzzer's crash-input or
Base64 text appearing in a public job log.

### SEC-012 — DMG creation dependency can panic and races a mutable source tree

- Severity: **Medium operationally** (potentially High when a privileged process reads an
  attacker-writable source tree)
- Status: **Panic contained locally; concurrent source mutation out of scope**
- CWE: CWE-367/CWE-248
- Regression: `dmg_creation_contains_path_dependency_panic` on Unix

`apple-dmg 0.5.0` uses UTF-8 path conversions that can panic for non-UTF-8 directory entries. DMG
creation now runs through the same dependency-panic boundary as filesystem parsing, preserving the
previous destination and returning a normal Python error for an unwinding panic.

The dependency still inspects a source entry and later opens it by pathname. Exploiting that gap
requires another process to mutate the source tree between those operations. The assessed model
states that the caller-controlled source tree is immutable, so this historical race is not an
active finding for this deployment.

### SEC-013 — Crafted GPT entry sizes reached a dependency assertion

- Severity: **Medium availability**
- Status: **Resolved locally; upstream assertion remains**
- CWE: CWE-248
- Regressions: `gpt_partition_entry_size_assertion_is_preflighted` and
  `gpt_partition_table_crc_mismatch_is_rejected`

`gpt 4.1.0` calls `assert_eq!(header.part_size, 128)` after accepting a CRC-valid primary or backup
header. A static malicious partition could therefore panic during `inspect_gpt()`. The local GPT
boundary now reads the same selected valid header first, requires 128-byte entries, caps the count
at 16,384, checks the partition-table range and CRC32, and wraps the dependency call in the common
panic boundary. A read-only borrowed device also replaces the prior full partition clone, reducing
peak memory by as much as 512 MiB. The dependency source itself was not modified.

### SEC-014 — Cyclic FAT chains could bypass entry-count budgets

- Severity: **Medium availability**
- Status: **Contained locally; upstream cycle detection remains absent**
- CWE: CWE-835/CWE-400
- Regression: `filesystem_io_budget_rejects_repeated_reads_and_excess_bytes`

`fatfs 0.3.6` follows cluster chains without a visited-cluster set. Its directory parser can loop
over deleted or long-name records without returning an entry, so the repository's 100,000-entry
counter was not guaranteed to run. Every local FAT entry point now uses a reader capped at 576 MiB
and 2,000,000 non-empty read operations, and FAT detection/listing/extraction all contain unwinding
dependency panics. A crafted cycle now terminates with an ordinary input error rather than hanging
indefinitely. The operation ceiling is intentionally separate from the output-byte limit.

### SEC-015 — Plist limits ran after dependency materialization

- Severity: **Medium availability**
- Status: **Resolved locally**
- CWE: CWE-789/CWE-674
- Regressions: `deeply_nested_plist_is_rejected_while_streaming` and
  `binary_plist_collection_allocation_is_preflighted`

The prior node/depth walk ran only after `plist::Value` had been built. In particular, a binary
array's declared length could drive `Vec::with_capacity` inside `plist` before the local limit was
checked. Binary trailer/object counts, collection lengths, and reference ranges are now validated
before parsing. XML, ASCII, and binary inputs then pass through a bounded event iterator before
value materialization, with the original post-build structural walk retained as defense in depth.
The dependency is constrained to the reviewed `~1.10.0` stream API because that API is explicitly
marked unstable across minor releases.

## Quality, test, CI, and documentation findings

### QA-001 — CI omitted the default `master` branch

- Status: **Resolved locally**

CI push filters now include both `master` and `main`; pull requests remain covered.

### QA-002 — Release tags could publish without quality or security gates

- Status: **Resolved locally**

Wheel and sdist jobs now depend on version verification, lint, strict typing, tests, dependency
audits, license policy, Python/native coverage, documentation, and four release fuzz-smoke jobs.
Publication depends on successful artifact jobs and retains least-privilege trusted publishing.

### QA-003 — Actions were mutable-tag pinned and permissions were broad

- Status: **Resolved locally**

Every action is pinned to a full SHA resolved from its official repository on 2026-07-18, with the
readable tag in a comment. Workflows default to `contents: read`; only publishing receives
`id-token: write`.

### QA-004 — Clippy failed on a large enum variant

- Status: **Resolved locally**

Large raw HFS+/APFS handles are boxed. `cargo clippy --locked --all-targets --all-features --
-D warnings` passes without a lint allowance.

### QA-005 — Bzip2, LZFSE, and ADC partition chunks were unsupported

- Status: **Bzip2/LZFSE resolved locally; ADC compatibility limitation external**
- Regressions: `compressed_decoders_round_trip_with_bounds` and
  `test_bzip2_partition_decoder_on_apfs_fixture`

Bounded bzip2 and LZFSE decoding is implemented. The committed APFS image's bzip2 GPT-header
partition decodes successfully. `apple-dmg` and `udif` both explicitly lack ADC decoding, so ADC
returns a precise unsupported-upstream error and remains an external feature limitation. It fails
closed before output and is not itself a security vulnerability.

### QA-006 — Native and adversarial regression tests were missing

- Status: **Resolved locally**

There are now 20 native tests on the reviewed Unix host (19 cross-platform) and 22 Python tests
covering BLKX count/truncation/layout, range overflow, zlib/bzip2/LZFSE decoding, expansion limits,
streamed/binary plist preflight, dependency panic containment, FAT read budgets, hostile HFS/APFS
block sizes, GPT assertion/count/range/CRC preflight, filesystem-partition budgets, streamed large
Zero/Ignore spans, checksum ranges, symlinks, atomic output, and real FAT/HFS+/APFS fixtures.
Sensitive crash bytes are not committed.

### QA-007 — Coverage was manual and unenforced

- Status: **Resolved and observed in hosted CI**

`pyproject.toml` enforces 90% Python line coverage. CI and release generate separate Python and
Rust reports and enforce 90% Python / 70% native line floors; CI uploads both. The local Python run
was 66/73 statements plus branch accounting (90%). Hosted run
[`29674156138`](https://github.com/bwhitn/pydmg/actions/runs/29674156138) at commit `1de7a87`
confirmed 90% Python coverage and 80.75% native lines, 60.22% functions, and 79.03% regions.
A clean 0.1.2 run with the pinned Rust 1.98.1 compiler reported 79.78% native lines, 59.19%
functions, and 77.82% regions. The optimized candidate's hosted run
[`35608047824`](https://github.com/bwhitn/pydmg/actions/runs/35608047824) reproduced exactly those
native totals and 90% Python coverage. The corrective 0.1.3 local and hosted release runs report
80.03% native lines and 90% Python coverage; all recorded native results remain above the gates.

### QA-008 — Fuzzing was not continuous

- Status: **Resolved; hosted PR smoke observed**

Five structure-aware targets, a dictionary, corpus builder, PR smoke matrix, weekly five-minute
ASan matrix, and release smoke matrix now exist. Direct-parser and whole-image jobs are gates. The
whole-image harness suppresses only the aborting libFuzzer panic hook so the production
`catch_unwind` boundary can run; an uncontained Rust panic still reaches libFuzzer's outer catch
and fails the job. Raw failure artifacts and crash-input log output are withheld from public CI per
SEC-011. Hosted run
[`29674156143`](https://github.com/bwhitn/pydmg/actions/runs/29674156143) passed the bounded `blkx`,
`chunk`, `dmg_parse`, and `gpt` ASan jobs; the slower whole-image job was skipped on the PR as
designed and remains a scheduled and release gate.

The optimized candidate's hosted run
[`35608047919`](https://github.com/bwhitn/pydmg/actions/runs/35608047919) also passed the bounded
`blkx`, `chunk`, `dmg_parse`, and `gpt` ASan jobs with the pinned dated nightly. Whole-image fuzz was
again skipped by the documented PR policy and remains mandatory in scheduled and release runs.

The 2026-09-21 post-optimization local campaigns then passed all five targets with
`nightly-2026-09-01`, cargo-fuzz 0.13.2, and AddressSanitizer: 15,698 `dmg_parse`, 457,781 `blkx`,
195,607 `chunk`, 444,612 `gpt`, and 3,327 whole-image executions. The image target completed its
full 300-second budget; no target produced a crash artifact, timeout, or sanitizer report.

### QA-009 — API and safety documentation was incomplete

- Status: **Resolved locally**

Every public function and `DmgImage` method has a docstring. README/SECURITY document `IndexError`,
resource ceilings, ADC, atomic replacement, symlink constraints, panic containment, CRC32 limits,
and residual sandbox requirements. `pydoc` generation passes.

### QA-010 — Advertised Python versions were not all tested

- Status: **Resolved locally**

Ubuntu tests Python 3.9, 3.10, 3.11, 3.12, and 3.13. Boundary versions also run on Windows and
macOS.

### QA-011 — Typing claims were not validated

- Status: **Resolved locally**

Strict mypy targets Python 3.9, runs over the package and a consumer-style fixture, and passes.
`_pydmg.pyi` describes the native extension. Dynamic JSON result schemas intentionally remain
`dict[str, Any]`/`list[dict[str, Any]]`; `Any` is confined with explicit casts at `json.loads`.

### QA-012 — Security tools were absent from CI and release

- Status: **Resolved locally**

Bandit, pip-audit, cargo-audit, exception-expiry validation, and the license policy are CI/release
gates. Tool versions that materially affect results are pinned in workflows.

### QA-013 — The Rust 1.88 minimum was untested

- Status: **Resolved locally**

CI checks and tests the locked all-feature graph with Rust 1.88.0 separately from the reproducible
Rust 1.98.1 primary toolchain. The current lock metadata declares no crate requiring a newer Rust
version.

### QA-014 — A local macOS wheel linked Homebrew `liblzma` dynamically

- Severity: **Medium packaging portability**
- Status: **Resolved locally**

The first locked release-wheel dry run warned that the extension required
`/usr/local/Cellar/xz/.../liblzma.5.dylib`. Release wheel jobs now set `LZMA_API_STATIC=1`, which
forces `lzma-sys` to build its bundled source, and pass `--auditwheel check` so an unexpected
external library fails the build. Non-Linux builds also request PyPI-compatible tags explicitly.

### QA-015 — The initial sdist omitted the nested fuzz crate

- Severity: **Low packaging/documentation consistency**
- Status: **Resolved locally**

Cargo's default package file set omitted the nested `fuzz/` crate even though the sdist included
`FUZZING.md`. Maturin now explicitly includes the fuzz manifest, five targets, and dictionary in
the sdist. The fuzz lockfile remains intentionally excluded; Cargo resolves that unpublished
harness independently.

### QA-016 — Hosted native coverage lacked a Python virtual environment

- Status: **Resolved and verified in hosted CI**

The first hosted coverage attempt passed the Python coverage gate but stopped before native
instrumentation because `maturin develop` could not find a virtual environment. CI and release
quality jobs now create `.venv`, install their Python tools through that interpreter, and publish
its binary directory through `GITHUB_PATH`. The native step also uses the current
`cargo llvm-cov show-env --sh` interface. The replacement hosted coverage job passed both floors.

### QA-017 — Pinned official actions declared the deprecated Node.js 20 runtime

- Severity: **Low operational compatibility**
- Status: **Resolved locally and guarded by hosted CI**

The successful post-merge `master` run
[`29674527731`](https://github.com/bwhitn/pydmg/actions/runs/29674527731) warned that the pinned
`actions/checkout` and `actions/setup-python` revisions declared Node.js 20, so GitHub forcibly ran
them on Node.js 24. Direct review of every pinned action descriptor found that the older
`actions/upload-artifact` and `actions/download-artifact` revisions also declared Node.js 20; the
remaining third-party actions were already composite or Node.js 24 actions. All workflow
occurrences now use immutable SHAs for the current official Node.js 24 releases: checkout 7.0.0,
setup-python 6.3.0, upload-artifact 7.0.1, and download-artifact 8.0.1. This removes reliance on the
hosted runner's compatibility override without weakening full-SHA pinning.

### QA-018 — The fuzz workflow did not validate changes to its own definition

- Severity: **Low gate coverage**
- Status: **Resolved locally and guarded by hosted CI**

The fuzz workflow's pull-request filter covered parser, corpus, fixture, and fuzz-harness changes
but omitted `.github/workflows/fuzz.yml`. A pull request that changed only its action pins therefore
started normal CI without exercising the updated fuzz jobs. The workflow now includes its own path,
so future changes to the fuzz gate trigger the four bounded PR smoke targets. Release-only action
pins are validated with a non-tag manual release run; the PyPI job remains restricted to version
tags.

### QA-019 — Primary builds and performance claims were not reproducible

- Status: **Resolved and observed in hosted CI**

`rust-toolchain.toml`, every primary CI job, release quality, and every wheel/sdist build now select
Rust 1.98.1 explicitly; the build backend and release action select maturin 1.14.1. The independent
Rust 1.88.0 MSRV job remains intact. `scripts/verify_versions.py` rejects primary-toolchain or MSRV
drift.

`scripts/benchmark.py` runs release scenarios in fresh worker processes and records wall/CPU time,
peak RSS, process block I/O, logical decoded/written bytes, Rust allocation/reallocation counts and
bytes, repository-owned staging copies, fixture hashes, startup time, and wheel/native sizes. The
dated two-warmup/nine-sample before/after evidence and profile decisions are in
[`PERFORMANCE.md`](PERFORMANCE.md). CI retains a five-sample SHA-named report; release retains nine
samples and blocks wheel builds until it succeeds. `--publish` refuses a dirty checkout, preventing
mutable working-tree measurements from being presented as published evidence.

Hosted PR run
[`35608047824`](https://github.com/bwhitn/pydmg/actions/runs/35608047824) passed the performance gate
from clean synthetic merge commit `5f284af72617cd2a489f5a5ecaaa63495df46b2f`. Its retained
five-sample report names that revision as both source and checkout, records `published: true`, and
has SHA-256 `d5f75273b8039df6674c44c557094aea31c3adce241a1cfbac4c02405ac063c3`.

### QA-020 — Implicit rustup components conflicted on some hosted images

- Severity: **Low operational compatibility**
- Status: **Resolved and observed in hosted CI**

The first Rust 1.98.1 pull-request run
[`35607307980`](https://github.com/bwhitn/pydmg/actions/runs/35607307980) passed lint, native,
coverage, security, MSRV, and most Python jobs, but three Python matrix entries and the performance
job stopped while rustup tried to add `rustfmt` and `clippy` implicitly. Those runner images already
contained the component binaries without matching component metadata, so rustup reported a
`bin/cargo-fmt` conflict. No project compilation or test failed.

`rust-toolchain.toml` now pins only the exact compiler and minimal profile; workflow jobs that need
rustfmt, Clippy, or LLVM tools install those components explicitly in the toolchain setup action.
MSRV commands use an explicit `+1.88.0` selector so the repository toolchain override cannot mask
the compatibility lane. Fuzz jobs install both the pinned stable compiler and
`nightly-2026-09-01` with `rust-src`, and invoke cargo-fuzz through that dated nightly, making each
toolchain role explicit.

The replacement CI run
[`35608047824`](https://github.com/bwhitn/pydmg/actions/runs/35608047824) passed every Linux, macOS,
and Windows matrix entry plus performance, coverage, security, lint, native, and explicit Rust 1.88
jobs. The paired fuzz run
[`35608047919`](https://github.com/bwhitn/pydmg/actions/runs/35608047919) passed all four PR ASan
targets. This confirms that the explicit toolchain setup resolves the hosted-image collision.

## Fuzzing evidence

Initial campaigns used `cargo-fuzz 0.13.2`, nightly Rust, sanitizer-instrumented standard library,
AddressSanitizer, valid fixture-derived seeds, and explicit input/time/RSS limits on x86_64 macOS.

| Target | Executions | Peak RSS | Result |
| --- | ---: | ---: | --- |
| `dmg_parse` | 29,337 | 730 MB | No failure in 61 s |
| `blkx` | 26 before stop | 47 MB | SEC-001 allocation finding |
| `chunk` | 862,367 | 410 MB | No failure in 61 s |
| `gpt` | 398,937 | 881 MB | No failure in 61 s |
| `image` | 50 before stop | 1,486 MB | SEC-002 upstream APFS panic; valid FAT seed took 13 s |

The direct targets exceeded 1.29 million executions. Campaigns were one minute, one host/OS, and
ASan only; they do not prove absence of defects. Exact commands and current CI behavior are in
[`FUZZING.md`](FUZZING.md).

After the whole-image harness was aligned with the production unwind boundary, the exact SEC-002
artifact completed as a fixed input (2 executions, 329 MB peak RSS). After all block and BLKX
preflights were added, a fresh 68-second whole-image ASan campaign completed 232 executions, grew
to 53 corpus units / 84 MiB, reached 5,334 coverage edges and 10,195 features, peaked at 1,496 MB
RSS, and found no new crash. Throughput was only about three executions per second, so longer
multi-platform campaigns remain necessary.

After the SEC-013, SEC-014, and SEC-015 boundary fixes, focused 30-second ASan campaigns produced:

| Target | Executions | Coverage / features | Peak RSS | Result |
| --- | ---: | ---: | ---: | --- |
| `gpt` | 70,439 | 695 / 1,386 | 462 MB | No crash, timeout, or sanitizer finding |
| `dmg_parse` | 17,096 | 1,574 / 3,548 | 515 MB | No crash, timeout, or sanitizer finding |
| `image` | 20 | 5,143 / 9,106 | 904 MB | No crash, timeout, or sanitizer finding |

The image campaign's valid compressed/filesystem seeds limited throughput to 20 executions in 35
seconds; it is useful post-change smoke evidence, not a substitute for the longer scheduled job.
Failure artifacts were directed to a newly created mode `0700` temporary directory; no repository
artifact was produced.

After the Rust 1.98.1 partition streaming and on-demand FAT changes, fresh local ASan campaigns on
2026-09-21 produced:

| Target | Budget | Executions | Coverage / features | Peak RSS | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| `dmg_parse` | 30 s | 15,698 | 1,619 / 3,978 | 528 MB | No finding |
| `blkx` | 30 s | 457,781 | 240 / 483 | 438 MB | No finding |
| `chunk` | 30 s | 195,607 | 954 / 2,287 | 391 MB | No finding |
| `gpt` | 30 s | 444,612 | 695 / 1,626 | 603 MB | No finding |
| `image` | 300 s | 3,327 | 5,661 / 12,561 | 1,059 MB | No finding |

These used cargo-fuzz 0.13.2, `nightly-2026-09-01`, owner-only corpus/artifact directories, and the
documented input/time/RSS ceilings. No crash artifact, timeout, or sanitizer report was produced.

## External issues and environment boundaries

The root causes or host state below were not modified outside this repository. Where possible, the
repository boundary now rejects or contains them. Items whose residual impact is solely CPU or
memory availability are retained for completeness but deferred under the current threat model:

1. `apfs 0.2.4` still needs B-tree key/value range validation, a safe key-count allocation, and
   B-tree depth/visited-node budgets (SEC-002). Local block/checkpoint preflight and unwind
   containment reduce impact but cannot stop allocator abort, stack overflow, or a hang. Current
   [upstream source](https://github.com/Dil4rd/dpp/blob/ea84058be933ce51e7b8251b302a3646daf8e12f/apfs/src/btree.rs)
   still contains the reviewed paths, and no matching public issue or fix was found.
2. `hfsplus 0.2.4` follows attacker-controlled B-tree child and forward-link nodes without a
   visited-node or traversal budget. Local block-size preflight prevents the obvious oversized
   allocation, and the shared dependency-read ceiling now terminates I/O-driven cycles. The
   dependency still lacks a semantic cycle/depth error and can consume substantial bounded CPU in the
   [dependency](https://github.com/Dil4rd/dpp/blob/ea84058be933ce51e7b8251b302a3646daf8e12f/hfsplus/src/btree.rs).
3. `udif 0.3.4` allocates whole gaps, compressed inputs, decompressed blocks, and a double-sized
   LZFSE temporary before the local writer sees output. Contiguous BLKX layout, 64 MiB expanded
   chunks, aggregate limits, and streaming local CRC32 now bound the reachable path, but the
   [dependency implementation](https://github.com/Dil4rd/dpp/blob/ea84058be933ce51e7b8251b302a3646daf8e12f/udif/src/reader.rs)
   itself remains allocation-heavy.
4. [`fatfs 0.3.6`](https://docs.rs/crate/fatfs/latest) remains the current release and still lacks
   visited-cluster detection, but repository-owned read-byte and
   read-operation budgets now stop cyclic static inputs and all local FAT boundaries contain
   unwinding panics (SEC-014).
5. [`gpt 4.1.0`](https://docs.rs/crate/gpt/latest) remains the current release and still asserts on
   a non-128-byte partition entry, but selected valid headers are now preflighted before that call
   and the dependency boundary also catches unwinding panics (SEC-013).
6. ADC decoding is absent from the upstream DMG decoders (QA-005). The local error is explicit and
   fail-closed; this is compatibility/availability, not a security vulnerability.
7. A pre-existing review artifact directory at `/private/tmp/pydmg-fuzz-artifacts` is mode `0755`
   with a minimized crash input mode `0644`. That is readable by other local users on a shared
   workstation. It was not changed because it is outside the repository; the operator should move
   sensitive artifacts to owner-only storage and use `umask 077` for future campaigns.
8. During the July review the normal workstation `PATH` exposed stable Cargo but not `rustup`,
   nightly, `cargo-fuzz`, or `cargo-llvm-cov`, so a temporary toolchain under `/private/tmp` supplied
   follow-up ASan evidence and hosted CI supplied the 80.75% native line result. As of the 2026-09-21
   review, rustup-managed Rust 1.98.1/1.88.0/nightly plus `cargo-fuzz` and `cargo-llvm-cov` are
   available locally. Final local and hosted Rust 1.98.1 native coverage are both 80.03%; the
   optimized and corrective commits passed their complete hosted cross-platform PR gates.
9. No upstream issue, pull request, or external message was created as part of this repository-only
   work. Upstream coordination is therefore still outstanding.

Two upstream behaviors previously listed as active security issues are out of scope for the
assessed deployment. [`apple-dmg 0.5.0`](https://docs.rs/crate/apple-dmg/latest/source/) remains
the current release; its pathname traversal would require concurrent mutation of a
caller-controlled creation tree, and the portable FAT path checks would require concurrent
mutation of the output tree. Both capabilities are excluded. The `apple-dmg` non-UTF-8 path panic
is still contained locally, and bulk FAT extraction retains its path defenses even though that API
is outside the narrower single-output workflow.

Two previously reported local/environment items are now resolved: the unnecessary
`dpp -> xara -> quick-xml 0.37.5` path was removed from `Cargo.lock`, and the ignored local `.venv`
was upgraded to `pip 26.2` after the active-environment audit identified PYSEC-2026-3721 in 26.1.2.
The repeated full audit reports no known vulnerabilities. CI/release continue to upgrade pip before
installing tools and audit both the project and complete tool environment.

## Completed local verification

The remediation working tree has passed:

```text
ruff check .
mypy
bandit -q -r python scripts
pytest -q                                      # 22 passed
coverage run -m pytest -q && coverage report  # 90%
cargo llvm-cov report --fail-under-lines 70   # 80.03% lines on Rust 1.98.1
cargo fmt --all -- --check
cargo fmt --manifest-path fuzz/Cargo.toml -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo clippy --manifest-path fuzz/Cargo.toml --bins -- -D warnings
cargo test --locked --all-targets --all-features  # 20 passed on macOS
cargo check --manifest-path fuzz/Cargo.toml --bins
rustup run 1.88.0 cargo test --locked --all-targets --all-features  # 20 passed
python scripts/check_audit_exceptions.py           # 0 exceptions
python scripts/check_licenses.py
python scripts/build_pydoc.py --cleanup
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
python scripts/verify_versions.py --expected 0.1.3
cargo audit                                    # 107 dependencies; no findings
python -m pip_audit .                         # no known vulnerabilities
python -m pip_audit                           # no known vulnerabilities
maturin build ... --locked --compatibility pypi --auditwheel check  # static liblzma
maturin sdist ... && twine check ...          # wheel and sdist passed
cargo-fuzz ASan: four 30-second parser campaigns and one 300-second image campaign
```

The local Cargo audit used `cargo-audit 0.22.1`, refreshed the RustSec database and crates.io index,
scanned 107 locked dependencies, and returned success with no ignores; workflows pin 0.22.2. The
other host limitations above are reported rather than bypassed.

The final clean-commit 0.1.2 packaging pass produced an 825,674-byte CPython 3.9+ ABI3 macOS wheel
(`ae73953919af65e090793c396f8d8be2ea2917f1da7dad9f8880aca07a423bc9`) and a 3,119,102-byte
source distribution (`28753317ae6187a6e788f1235ac0e88cf0531d38d006ad3b9e0c604aed57da36`).
Auditwheel and Twine accepted both artifacts, an isolated install parsed the committed FAT fixture,
and the wheel's native dependency list contained only system libraries because liblzma was linked
statically.

The replacement hosted PR CI run
[`35608047824`](https://github.com/bwhitn/pydmg/actions/runs/35608047824) passed the Python/native
coverage job, dependency-security job, clean-SHA performance job, lint/type/documentation job,
native and Rust 1.88 jobs, and the Python 3.9–3.13 Linux, macOS, and Windows matrix. Its coverage
artifact records 90% Python and 79.78% native lines. Its published performance artifact has SHA-256
`d5f75273b8039df6674c44c557094aea31c3adce241a1cfbac4c02405ac063c3`. Paired fuzz run
[`35608047919`](https://github.com/bwhitn/pydmg/actions/runs/35608047919) passed all four PR
fuzz-smoke jobs; whole-image fuzz was skipped by its documented PR policy.

## 0.1.3 corrective compatibility verification

After the immutable 0.1.2 release was published, downstream acceptance against an authorized Apple
filesystem image exposed a compatibility regression in the new BLKX preflight. One valid
`Ignore` chunk represented 82,345,984 expanded bytes, which exceeded the 64 MiB per-decoder-chunk
limit even though `Ignore` and `Zero` chunks are emitted incrementally from a fixed 128 KiB buffer.
The complete partition remained below the independent 512 MiB partition limit.

Version 0.1.3 exempts only streamed `Ignore` and `Zero` spans from the dependency-allocation limit.
Compressed and raw chunks retain the 64 MiB expanded-size guard, and every chunk remains covered
by checked sector arithmetic and the aggregate partition limit. A deterministic metadata-only
regression accepts a streamed span one sector above 64 MiB without materializing it; the adjacent
compressed-chunk regression proves that decoder allocations above the same boundary are still
rejected. No authorized corpus bytes, names, or hashes are committed.

The corrective working tree passed all repository gates: 22 Python tests, 20 Rust tests on Rust
1.98.1 and 1.88.0, 90% Python line coverage, 80.03% native line coverage, Ruff, mypy, Bandit, both
rustfmt checks, root/fuzz Clippy, fuzz builds, documentation, version, license, Cargo audit, and both
pip-audit forms. Five bounded AddressSanitizer campaigns completed without a crash, timeout,
sanitizer report, or artifact; their statistics are retained in [`FUZZING.md`](FUZZING.md). An
isolated install of the production-style wheel parsed both the committed FAT fixture and the
authorized acceptance image, reporting two and seven partitions respectively.

The local static-liblzma artifacts passed auditwheel and Twine. The macOS ABI3 wheel is 825,684
bytes with SHA-256 `f23103bc5eabba23bb0319d2ac9a80f2bd5a20ce0ae976276328cad8b5f68422`; the
3,120,635-byte source distribution has SHA-256
`c82125c75c862ab94a7c1a6daadd897ce118b9857028195d94cd122ff9d766d3`.

Corrective PR [`#4`](https://github.com/bwhitn/pydmg/pull/4) passed complete hosted CI run
[`35617605668`](https://github.com/bwhitn/pydmg/actions/runs/35617605668), including Python 3.9–3.13
Linux/macOS/Windows, Rust 1.88 MSRV, 90% Python coverage, 80.03% native line coverage, audits,
documentation, and clean-SHA performance. Paired fuzz run
[`35617605544`](https://github.com/bwhitn/pydmg/actions/runs/35617605544) passed all four direct-parser
jobs. PR #4 merged as `36c6635369652f43a6f88211e905873e51bc1148`; manual release preflight
[`35618182500`](https://github.com/bwhitn/pydmg/actions/runs/35618182500) passed before that exact
revision was tagged `v0.1.3`.

Trusted release run [`35619257967`](https://github.com/bwhitn/pydmg/actions/runs/35619257967)
passed every quality, security, performance, direct-parser fuzz, sdist, five-wheel, metadata, and
PyPI publishing job. The non-yanked public artifacts are:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| macOS x86-64 ABI3 wheel | 821,843 | `cf180be7ab1e91e4a4df56e1a7b2e958599a3886376902d83dd3284c7ee2c6e8` |
| macOS arm64 ABI3 wheel | 830,643 | `4828d8e8100dfb563050eb76000159b62bd791ea685baf419278c4e75641e848` |
| manylinux aarch64 ABI3 wheel | 935,557 | `b1f1977fa7c4751a20bbe7c9a2484658c4cf4208c7053f5706dd9bd992190c92` |
| manylinux x86-64 ABI3 wheel | 930,413 | `27055d1875c6ead8500fa0fc62856cf23698615db745359bf909e4212ed73485` |
| Windows x86-64 ABI3 wheel | 722,846 | `ac600e8a84ab988e1cfad8aacfc80044ba0e9b664f4ff28775e368f9e0568c6a` |
| Source distribution | 3,121,652 | `a7cdca2ec85e0bfb70a5dc8e84cdc2e1e264b27ac0a91c53b98503e783d438b0` |

A fresh isolated wheel-only install from public PyPI parsed the committed FAT fixture and the
authorized Apple image, reporting two and seven partitions. ALES exact-pins `pydmg==0.1.3`, locks
the six hashes above, and retains the license, third-party notices, and CycloneDX SBOM through its
stripped/pruned runtime. Its host and contained test-image suites each pass all 1,959 tests. The
exact operational image `sha256:436d94394841b39bf5107c7a638d59af909c9c9ddb66cc280df93da127f67c35`
preserves the authorized image's 21-record type profile with no error, exact pydmg DMG report,
ObjectRules map, and stored-artifact paths and bytes; wall time improved from 36.72 to 30.92
seconds versus the 0.1.1 baseline. The generated FAT acceptance preserves its expected eight
records, ObjectRules map, and stored-artifact bytes while updating parser diagnostic wording; wall
time improved from 8.83 to 3.59 seconds. No authorized corpus bytes, names, or hashes are recorded.

## Deferred and non-blocking follow-up

None of these items is an active security blocker under the assessed threat model:

1. Privately report the APFS panic/allocation/traversal defects and HFS traversal-cycle defect;
   adopt fixed dependency releases after the private regressions and sanitizer jobs pass.
2. Coordinate upstream on UDIF streaming allocations; until then, keep the local allocation bounds
   and host memory limit.
3. Consider reporting the contained `fatfs` cycle and `gpt` assertion defects upstream so the local
   guards can eventually become defense in depth rather than the primary boundary.
4. Continue longer scheduled and multi-platform sanitizer campaigns, and raise coverage floors as
   stable coverage grows.
5. Add ADC only as an optional compatibility improvement through a reviewed bounded
   implementation or upstream support.
