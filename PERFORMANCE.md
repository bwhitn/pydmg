# Performance Evidence

## 2026-09-21 Rust 1.98.1 optimization

The immutable baseline is commit `564fc270827fc7e5453524e4509ed6ac135fddaa`. The primary
before/after measurements below are local pre-commit evidence and are intentionally marked
unpublished in the machine-readable result. Clean-commit and hosted evidence is recorded after the
comparison. CI and release workflows rerun the suite from a clean checkout and
`scripts/benchmark.py --publish` refuses a dirty tree, so retained published artifacts are tied to
the exact Git commit SHA.

### Method

- Compiler: `rustc 1.98.1 (48a229cea 2026-09-01)` / Cargo 1.98.1, release profile;
  maturin 1.14.1.
- Host: x86-64 macOS 26.5.2, CPython 3.12.10.
- Sampling: two warmups followed by nine measured runs per scenario; every run used a fresh process.
- Fixtures: the generated 512 MiB FAT32 UDIF and the bzip2-compressed APFS UDIF already recorded in
  `tests/fixtures/README.md`. The benchmark report verifies their committed SHA-256 values.
- Timing uses monotonic wall time and process CPU time. Peak RSS is the fresh worker's process peak.
  Rust allocations are counted by the opt-in `benchmarking` allocator; CPython and third-party C
  allocator calls are not included. Copy counts cover repository-owned partition staging copies.
- `ru_inblock` and `ru_oublock` both remained zero because the fixtures were warm in the host cache.
  Logical decoded and sink-written bytes are therefore recorded separately and are the useful I/O
  measures for this run.

Raw samples, min/mean/median/standard deviation/p95/max, environment metadata, artifact sizes, and
fixture hashes are in
[`baseline-macos-x86_64.json`](benchmarks/results/baseline-macos-x86_64.json) and
[`optimized-macos-x86_64.json`](benchmarks/results/optimized-macos-x86_64.json).

### Results

Median wall time:

| Scenario | Baseline | Optimized | Change |
| --- | ---: | ---: | ---: |
| Import/startup | 5.175 ms | 4.560 ms | -11.9% observed; startup cache noise dominates |
| Trailer/plist/BLKX projection | 2.380 ms | 2.225 ms | -0.155 ms; below a meaningful effect size |
| Streaming CRC32 | 0.884 ms | 0.870 ms | -0.014 ms; below a meaningful effect size |
| Read 20.9 MiB compressed partition | 325.618 ms | 276.526 ms | -15.1% |
| Atomically extract that partition | 336.488 ms | 312.013 ms | -7.3% |
| List generated 512 MiB FAT32 partition | 413.521 ms | 1.225 ms | -99.70% (338x) |
| Extract two FAT32 fixture files | 452.728 ms | 41.905 ms | -90.7% |
| List APFS directory | 299.333 ms | 278.591 ms | -6.9% observed; unchanged code path |
| Atomically extract APFS `Info.plist` | 324.729 ms | 298.761 ms | -8.0% observed; unchanged code path |

The large effects remain well outside run-to-run variation. For example, FAT listing was
409.240–417.190 ms before and 1.209–2.167 ms after; FAT extraction was 446.499–459.145 ms before
and 40.789–43.430 ms after. The APFS ranges overlap and that dependency-owned code did not change,
so its observed movement is not attributed to this optimization.

Median resource evidence for the changed paths:

| Scenario | Peak RSS before / after | Rust allocated bytes before / after | Staging copies before / after |
| --- | ---: | ---: | ---: |
| Compressed partition read | 89.1 / 44.0 MiB | 24.8 MiB / 133 KiB | 20.9 MiB / 0 |
| Partition extraction | 68.2 / 23.3 MiB | 24.8 MiB / 133 KiB | 20.9 MiB / 0 |
| FAT listing | 533.2 / 20.9 MiB | 550.8 / 1.28 MiB | 512 MiB / 1.66 KiB |
| FAT extraction | 533.3 / 20.9 MiB | 550.8 / 1.28 MiB | 512 MiB / 1.68 KiB |

FAT listing and extraction previously decoded 512 MiB from 532,531 compressed bytes on every call.
The seekable reader decoded only the two requested 1 MiB chunks (2 MiB from 2,641 compressed bytes)
for these fixtures. Logical user output was unchanged: 244 JSON bytes for listing and 21 extracted
file bytes. Partition extraction still decoded and atomically wrote all 21,942,272 bytes.

A separate differential run canonicalized partition metadata, FAT/APFS filesystem metadata and
listings, the 20.9 MiB APFS partition payload, every extracted FAT fixture path/payload, and the
APFS `Info.plist` payload. Baseline and 0.1.2 candidate output were byte-for-byte identical
(SHA-256 `dbbf2bfc1d08ac4989ed6138273189ba08e15b07e70172610693d94619c0fd25`).

After committing the code candidate, a second nine-sample run used the final static-liblzma wheel
from a clean worktree. Its report records both `source_revision` and `checked_out_revision` as
`27edd356536837c44ed9d309eb34c17355cbbd04`, with `published: true`; the report SHA-256 is
`3fc99a17452b874b78a4b9d78e6d888fd1fe40de0775c0cc4cf8ddd81be203c5`. Median FAT listing was
1.504 ms (274.9x faster than baseline) and FAT extraction was 41.956 ms (-90.7%). APFS listing and
extraction were 302.267 ms and 321.394 ms, respectively, both inside the baseline ranges. This
confirms the material effects and the absence of a material regression in the unchanged APFS path
using the exact clean candidate source.

Hosted PR run
[`35608047824`](https://github.com/bwhitn/pydmg/actions/runs/35608047824) repeated five samples from
clean synthetic merge commit `5f284af72617cd2a489f5a5ecaaa63495df46b2f`. Its retained report
records matching source and checkout revisions, `working_tree_dirty: false`, and `published: true`;
the report SHA-256 is `d5f75273b8039df6674c44c557094aea31c3adce241a1cfbac4c02405ac063c3`.
Median FAT listing and extraction were 1.131 ms and 2.804 ms on the hosted Linux runner, while APFS
listing and extraction were 279.269 ms and 279.073 ms. The hosted performance gate passed.

The final 0.1.3 tag-gated [release run](https://github.com/bwhitn/pydmg/actions/runs/35619257967)
repeated all nine scenarios with two warmups and nine measured samples on hosted x86-64 Linux. Its
retained report records both source and checkout revisions as
`36c6635369652f43a6f88211e905873e51bc1148`, `working_tree_dirty: false`, and `published: true`.
The report SHA-256 is `fc97decdf5d6d6a6c22f01d9f6725c0a4f4fbe4aaa6903ab8aa85b1b55a92c08`.

| Release scenario | Median wall time |
| --- | ---: |
| Import/startup | 2.858 ms |
| Trailer/plist/BLKX projection | 1.059 ms |
| Streaming CRC32 | 0.653 ms |
| Read 20.9 MiB compressed partition | 268.464 ms |
| Atomically extract that partition | 295.262 ms |
| List generated 512 MiB FAT32 partition | 1.119 ms |
| Extract two FAT32 fixture files | 3.080 ms |
| List Apple-filesystem directory | 280.827 ms |
| Atomically extract the Apple fixture file | 281.666 ms |

The measured manylinux wheel is 933,805 bytes and its production native extension is 2,412,904
bytes. These final numbers preserve the large FAT improvement and keep the unchanged Apple path in
the established hosted range after the narrow streamed Zero/Ignore compatibility correction.

The production macOS wheel grew from 775,910 to 785,256 bytes (+1.20%); its uncompressed native
extension grew from 1,814,344 to 1,975,224 bytes (+8.87%). This is the measured size cost of the
bounded seekable partition reader and streaming decoder paths. The opt-in allocator instrumentation
is not present in those production artifacts. Those like-for-like benchmark wheels used the same
local linking mode. Separately, the final static-liblzma 0.1.2 release-candidate wheel is 825,674
bytes; its packaging checks and digest are recorded in [`AUDIT.md`](AUDIT.md).

### Profile findings and decisions

The baseline FAT sample attributed 2,564 of 3,595 on-CPU samples to `memmove` under
`read_partition_bytes`, with repeated output-vector growth also visible. Its peak physical footprint
reached 790 MiB during the repeated profiling loop. The replacement:

- exposes a read-only, seekable BLKX partition device to `fatfs` and keeps at most one bounded
  compressed chunk decoded;
- streams raw, zlib, and bzip2 data directly to the final sink, writes zero/ignore spans from a fixed
  buffer, and reuses LZFSE compressed/output buffers;
- writes public `bytes` directly into the pre-sized Python object and partition extraction directly
  into the sibling temporary file; and
- requires every complete partition decode to produce its declared expanded byte count.

The APFS profile attributed 3,875 of 4,033 samples to `udif` partition decompression, including
2,581 samples in the system bzip2 decoder. That path was left unchanged: replacing dependency-owned
filesystem extraction was not justified by the flat before/after results and would expand the
security boundary.

Trailer/plist/BLKX parsing, checksum work, GPT traversal, filename conversion, and JSON/PyO3
projection were measured but were not material hot spots. Rust 1.98's new `String::from_utf16le`
and `String::from_utf16be` helpers were evaluated but not adopted. GPT and HFS+ dependencies own the
raw UTF-16 fields and currently expose already-decoded, lossy `String` values; the repository does
not receive the original byte slices. Re-decoding locally would duplicate parsing, and switching to
the strict helpers would change malformed-name behavior. No registry dependency was edited.

### Reproduce

Build production size evidence, then the instrumented extension and benchmark it:

```bash
maturin build --release --locked --out dist
maturin develop --release --locked --features benchmarking
python scripts/benchmark.py \
  --label local \
  --samples 9 \
  --warmups 2 \
  --wheel dist/pydmg-*.whl \
  --output performance.json
```

Use `--publish` only from a clean checkout. The flag rejects mutable source state. CI runs five
samples for regression evidence; the tag-gated release workflow runs the nine-sample suite before
building distributable artifacts and names the retained report with `${GITHUB_SHA}`.
