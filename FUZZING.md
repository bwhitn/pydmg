# Fuzzing Guide

The fuzz suite uses `cargo-fuzz`, LLVM libFuzzer, coverage-guided mutation, debug assertions, and
AddressSanitizer. It deliberately separates outer-container parsing from deeper parsers so an
invalid DMG trailer cannot prevent BLKX, chunk, GPT, or filesystem code from receiving coverage.

## Targets

| Target | Input | Code exercised | Primary failure classes |
| --- | --- | --- | --- |
| `dmg_parse` | Complete DMG bytes | `koly`, plist, metadata, and embedded BLKX parsing | panic, integer overflow, excessive allocation, parser hang |
| `blkx` | Raw `mish`/BLKX table | chunk-table count and field parsing | excessive allocation, truncation handling, panic |
| `chunk` | Structured header plus compressed payload | raw, zero, zlib, bzip2, LZFSE, ADC rejection, terminal, and unknown chunk paths | decompression bomb, panic, out-of-bounds access, hang |
| `gpt` | Raw disk/partition bytes | 512- and 4096-byte GPT parsing and validation | panic, excessive allocation, arithmetic error, hang |
| `image` | DMG or raw filesystem image | full inspection, FAT recursion, HFS+/APFS root listing, and representative file reads | cross-parser crash, recursion failure, resource exhaustion, dependency sanitizer finding |

The `chunk` harness bounds synthetic zero-fill sectors, keeps compressed ranges inside the fuzz
input, and invokes the same checked decoder used by production. The image harness caps the number
of partitions and files attempted per iteration. Production byte, count, nesting, decompression,
and aggregate-output limits remain active, so mutations exercise both successful paths and safe
rejection boundaries.

## Prerequisites

```bash
rustup toolchain install nightly --profile minimal --component rust-src
cargo install cargo-fuzz --version 0.13.2 --locked
python -m pip install -e ".[dev]"
```

The nightly and `rust-src` components are required for libFuzzer and for sanitizer-instrumenting the
Rust standard library. On macOS or Linux, use a supported Clang/LLVM toolchain. Run fuzzers on a
disposable development machine or isolated runner, never on a production host.

## Prepare the structure-aware corpus

```bash
umask 077
PYDMG_FUZZ_CORPUS="$(mktemp -d)"
python scripts/prepare_fuzz_corpus.py --output-dir "${PYDMG_FUZZ_CORPUS}"
```

The corpus builder:

- copies the committed FAT32 and APFS DMGs;
- decompresses the committed raw HFS+ fixture;
- extracts BLKX tables directly from each DMG plist;
- extracts reasonably sized partition payloads for GPT mutation;
- creates raw, zero-fill, valid-zlib, and valid-bzip2 chunk seeds (the native unit suite supplies a
  positive LZFSE round-trip); and
- creates a protective-MBR/primary-GPT/backup-GPT seed with one partition.

Fixture provenance and SHA-256 values are recorded in
[`tests/fixtures/README.md`](tests/fixtures/README.md). Generated corpus and artifact files belong in
an owner-only temporary directory and should not be committed. Keep the `umask 077` setting for the
entire campaign because a minimized crash may be security-sensitive.

## Bounded smoke campaigns

The following commands run each target for five minutes. Increase `-max_total_time` to hours for
parser releases or dependency upgrades.

The 2026-07-18 baseline found failures in `blkx` and `image`; their hashes and impact are in
[`AUDIT.md`](AUDIT.md). The BLKX target now enters through the local count/layout/allocation guard
and has deterministic regressions. The image failure originates in `apfs 0.2.4`: normal Python
entry points contain its unwinding panic as `RuntimeError`, but the dependency root cause remains
open. The whole-image target replaces libFuzzer's aborting panic hook so the production catch can
run; an uncontained panic still reaches libFuzzer's outer catch and fails the process. The exact
artifact and a post-preflight 68-second ASan campaign now complete without a new failure.

```bash
cargo +nightly fuzz run --sanitizer address --build-std dmg_parse \
  "${PYDMG_FUZZ_CORPUS}/dmg_parse" -- \
  -max_total_time=300 -max_len=4194304 -timeout=10 -rss_limit_mb=3072 \
  -dict=fuzz/dictionaries/dmg.dict -print_final_stats=1

cargo +nightly fuzz run --sanitizer address --build-std blkx \
  "${PYDMG_FUZZ_CORPUS}/blkx" -- \
  -max_total_time=300 -max_len=1048576 -timeout=5 -rss_limit_mb=2048 \
  -dict=fuzz/dictionaries/dmg.dict -print_final_stats=1

cargo +nightly fuzz run --sanitizer address --build-std chunk \
  "${PYDMG_FUZZ_CORPUS}/chunk" -- \
  -max_total_time=300 -max_len=1048576 -timeout=5 -rss_limit_mb=2048 \
  -dict=fuzz/dictionaries/dmg.dict -print_final_stats=1

cargo +nightly fuzz run --sanitizer address --build-std gpt \
  "${PYDMG_FUZZ_CORPUS}/gpt" -- \
  -max_total_time=300 -max_len=4194304 -timeout=10 -rss_limit_mb=2048 \
  -dict=fuzz/dictionaries/dmg.dict -print_final_stats=1

cargo +nightly fuzz run --sanitizer address --build-std image \
  "${PYDMG_FUZZ_CORPUS}/image" -- \
  -max_total_time=300 -max_len=4194304 -timeout=30 -rss_limit_mb=4096 \
  -dict=fuzz/dictionaries/dmg.dict -print_final_stats=1
```

`-max_total_time` is a campaign budget, not a per-input timeout. Keep explicit input, memory, and
per-input time limits so hangs and allocation regressions become artifacts rather than destabilizing
the fuzz host.

The repository runs the four direct-parser targets for pull requests, weekly, and before a release
in `.github/workflows/fuzz.yml` and `.github/workflows/release.yml`. Scheduled whole-image fuzzing
is also a required gate. Workflow failure output is captured to a runner-local file so libFuzzer
does not print a crash input or Base64 reproducer into the public log.

## Reproduce and minimize a finding

`cargo-fuzz` writes failures under `fuzz/artifacts/<target>/` unless `-artifact_prefix` overrides the
location.

```bash
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<artifact>
cargo +nightly fuzz tmin <target> fuzz/artifacts/<target>/<artifact>
```

For each unique finding:

1. preserve the original and minimized artifact outside the mutable corpus in an owner-only
   directory;
2. record the toolchain, sanitizer, target, arguments, stack trace, and SHA-256;
3. determine whether the crash is in `pydmg`, a Rust dependency, or bundled native code;
4. add a deterministic regression test before or with the fix; and
5. rerun every target because parser fixes often move the reachable frontier.

Do not commit a sensitive or weaponized artifact. Follow [`SECURITY.md`](SECURITY.md) for private
reporting. Do not upload raw crash directories from a public GitHub Actions job: public-repository
readers can download artifacts, and libFuzzer failure output can itself contain a reproducer or
Base64 input. Reproduce a CI failure on a private runner before preserving or sharing the input.

## Coverage and campaign quality

Execution count alone is not enough. A useful campaign must start from valid samples, show corpus
growth, and reach successful parser paths as well as rejection paths. Generate a coverage report
after corpus minimization:

```bash
cargo +nightly fuzz cmin <target> "${PYDMG_FUZZ_CORPUS}/<target>"
cargo +nightly fuzz coverage <target> "${PYDMG_FUZZ_CORPUS}/<target>"
```

The dated baseline and its limitations are recorded in [`AUDIT.md`](AUDIT.md). A bounded clean run
means only that no finding occurred during that campaign; it is not a security guarantee.

On 2026-09-21, after the Rust 1.98.1 streaming/on-demand partition changes, all five targets ran
with cargo-fuzz 0.13.2, `nightly-2026-09-01`, AddressSanitizer, structure-aware seeds, and owner-only
corpus/artifact directories:

| Target | Budget | Executions | Coverage / features | Peak RSS | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| `dmg_parse` | 30 s | 15,698 | 1,619 / 3,978 | 528 MB | No finding |
| `blkx` | 30 s | 457,781 | 240 / 483 | 438 MB | No finding |
| `chunk` | 30 s | 195,607 | 954 / 2,287 | 391 MB | No finding |
| `gpt` | 30 s | 444,612 | 695 / 1,626 | 603 MB | No finding |
| `image` | 300 s | 3,327 | 5,661 / 12,561 | 1,059 MB | No finding |

No crash artifact, timeout, or sanitizer report was produced. These are bounded single-host
campaigns; the scheduled and release jobs remain the canonical recurring gates.

## Known instrumentation limits

- Native libraries built by dependency build scripts may not receive the same sanitizer coverage as
  Rust code unless their build scripts propagate sanitizer compiler flags.
- Out-of-memory termination can occur before libFuzzer writes a useful artifact. Preserve the last
  reported unit and rerun with stricter host limits when investigating.
- The filesystem target is slower than pure in-memory targets and needs a longer per-input timeout.
- Fuzzing one host architecture does not replace campaigns on other supported architectures and
  operating systems.
- During the July review, a temporary nightly/cargo-fuzz toolchain under `/private/tmp` supplied the
  exact-input, 68-second follow-up, and focused post-boundary ASan runs because the normal `PATH`
  lacked those tools. The latter completed 70,439 `gpt`, 17,096 `dmg_parse`, and 20 `image`
  executions without a crash, timeout, or sanitizer finding. The whole-image seeds are expensive,
  so that 35-second image run is only smoke evidence. As of 2026-09-21, rustup nightly,
  `cargo-fuzz`, and `cargo-llvm-cov` are available locally. A fresh Rust 1.98.1 native run now
  records 79.78% line coverage; the pinned hosted job's last recorded result remains 80.75%.
