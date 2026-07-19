# Code Quality And Security Audit

Assessment date: **2026-07-18**
Remediation review: **2026-07-19**
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
The current local suite passes 21 Python integration/adversarial tests and 17 native Rust tests
(16 cross-platform plus one Unix path-panic regression).
Ruff, strict mypy, Bandit, rustfmt, and Clippy with warnings denied are green. Python line coverage
remains 90% and now has a gate; hosted native coverage is 80.75% lines against a separate 70%
workflow gate.

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
| Python tests | 21 passed |
| Native Rust tests | 17 passed on macOS; 16 cross-platform |
| Python coverage | 90%; enforced at 90% in CI/release |
| Native Rust coverage | Hosted result 80.75% lines; enforced at 70% in CI/release |
| Python lint/type | Expanded Ruff rules, strict mypy, and consumer stub check pass |
| Security lint | Bandit passes; three exact false positives have inline rationale |
| Rust lint | rustfmt and Clippy `-D warnings` pass |
| Fuzz harness | Five targets compile; four hosted PR ASan smoke jobs passed; whole-image fuzz runs on schedule/release |
| Python dependency audit | No runtime dependencies or known vulnerability in the last completed run |
| Rust dependency audit | PyO3 and quick-xml advisories resolved; no configured exceptions remain |
| Documentation | README, API docstrings, security policy, audit, fuzz guide, and `AGENTS.md` updated |
| Release readiness | Repository gates are present; resource risks are deferred and ADC is a compatibility limitation |

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

Zlib and bzip2 decode through a one-byte-over-limit reader; LZFSE receives a bounded declared-size
buffer; zero/raw chunks and aggregate partition output are checked before allocation. Binary plist
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

There are now 17 native tests on the reviewed Unix host (16 cross-platform) and 21 Python tests
covering BLKX count/truncation/layout, range overflow, zlib/bzip2/LZFSE decoding, expansion limits,
streamed/binary plist preflight, dependency panic containment, FAT read budgets, hostile HFS/APFS
block sizes, GPT assertion/count/range/CRC preflight, filesystem-partition budgets, checksum ranges,
symlinks, atomic output, and real FAT/HFS+/APFS fixtures. Sensitive crash bytes are not committed.

### QA-007 — Coverage was manual and unenforced

- Status: **Resolved and observed in hosted CI**

`pyproject.toml` enforces 90% Python line coverage. CI and release generate separate Python and
Rust reports and enforce 90% Python / 70% native line floors; CI uploads both. The local Python run
was 66/73 statements plus branch accounting (90%). Hosted run
[`29674156138`](https://github.com/bwhitn/pydmg/actions/runs/29674156138) at commit `1de7a87`
confirmed 90% Python coverage and 80.75% native lines, 60.22% functions, and 79.03% regions.

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

CI checks and tests the locked all-feature graph with Rust 1.88.0 separately from moving stable.
The current lock metadata declares no crate requiring a newer Rust version.

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
8. The normal workstation `PATH` has stable Cargo but not `rustup`, nightly, `cargo-fuzz`, or
   `cargo-llvm-cov`. A prior temporary nightly/cargo-fuzz toolchain was recoverable under
   `/private/tmp` and was used for the follow-up ASan and focused boundary runs. Post-remediation
   native coverage remains unavailable locally because `cargo-llvm-cov` is absent; pinned CI owns
   that assurance check and has recorded an 80.75% hosted line result.
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
was upgraded from vulnerable `pip 25.0.1` to `pip 26.1.2`. A full active-environment pip audit now
reports no known vulnerabilities. CI/release continue to upgrade pip before installing tools and
audit both the project and complete tool environment.

## Completed local verification

The remediation working tree has passed:

```text
ruff check .
mypy
bandit -q -r python scripts
pytest -q                                      # 21 passed
coverage run -m pytest -q && coverage report  # 90%
cargo fmt --all -- --check
cargo fmt --manifest-path fuzz/Cargo.toml -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo clippy --manifest-path fuzz/Cargo.toml --bins -- -D warnings
cargo test --locked --all-targets --all-features  # 17 passed on macOS
cargo check --manifest-path fuzz/Cargo.toml --bins
python scripts/check_audit_exceptions.py           # 0 exceptions
python scripts/check_licenses.py
python scripts/build_pydoc.py --cleanup
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
python scripts/verify_versions.py --expected 0.1.0
cargo audit                                    # 107 dependencies; no findings
python -m pip_audit .                         # no known vulnerabilities
python -m pip_audit                           # no known vulnerabilities
maturin build ... --locked --auditwheel check
maturin sdist ... && twine check ...          # wheel and sdist passed
```

The Cargo audit used pinned `cargo-audit 0.22.2`, refreshed the RustSec database and crates.io
index, scanned 107 locked dependencies, and returned success with no ignores. The other host
limitations above are reported rather than bypassed.

The replacement hosted PR run passed the Python/native coverage job, dependency-security job,
lint/type/documentation job, native and Rust 1.88 jobs, and the Python 3.9–3.13 Linux, macOS, and
Windows matrix. All four PR fuzz-smoke jobs also passed; whole-image fuzz was skipped by its
documented PR policy.

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
