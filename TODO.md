# pydmg TODO

## Current

- [x] Adopt Rust 1.98.1 and optimize measured DMG analysis/extraction hot paths — **Complete in the published 0.1.3
  release, including hosted release evidence and ALES integration acceptance**:
  - [x] Capture release-mode baselines for generated UDIF/plist/BLKX, compressed-chunk, partition, filesystem, and
    extraction fixtures. Record wall time, CPU, peak memory, allocations/copies, decompressed and written bytes,
    disk I/O, startup/import time, and wheel/native size. `PERFORMANCE.md` and `benchmarks/results/` contain the local
    nine-sample baseline/candidate evidence, fixture hashes, artifact sizes, and reproduction commands.
  - [x] Prepare local toolchain and workflow changes that select Rust 1.98.1 while retaining Cargo
    `rust-version = "1.88"` and a separate MSRV lane. These changes are locally validated; clean-SHA and hosted
    release evidence is tracked separately below.
  - [x] Profile trailer/plist/BLKX parsing, chunk scheduling and decompression, checksum work, partition/filesystem
    traversal, filename conversion, sink writes, and PyO3 projection. Evaluate Rust 1.98 UTF-16BE/LE helpers only
    where the on-disk encoding and malformed-input behavior match. The local candidate adds a bounded seekable BLKX
    partition reader, streaming sink paths, reusable LZFSE buffers, and direct Python/output writes; it deliberately
    leaves dependency-owned APFS decompression and already-decoded filename behavior unchanged.
  - [x] Reconcile the working branch before validation without discarding any owner work:
    - [x] Review every modified and untracked file on `agent/actions-node24`, whose published head is still
      `564fc270827fc7e5453524e4509ed6ac135fddaa`, and separate modernization changes from unrelated branch work. All
      source, test, workflow, benchmark, and documentation changes belong to this initiative; the owner-authored
      checklist was preserved and updated only with verified status/evidence.
    - [x] Integrate the current default-branch history using a non-destructive owner-approved merge/rebase strategy,
      resolve conflicts explicitly, and regenerate the benchmark comparison if the effective baseline or candidate
      source changes. Remote `master` is `4b2fee4764375e696b5159c47f9ae5db3c08d339`; its tree is byte-for-byte identical
      to the published branch head, but the two equivalent commits had different ancestry. Owner-approved merge
      `ea8039de944193311e00d0d9dea0b2530d968b5e` joined both histories without rewriting the published branch or changing
      any file, so no benchmark regeneration was needed for the reconciliation itself.
    - [x] Reconcile repository metadata currently declaring `0.1.0` with the `pydmg==0.1.1` release already consumed
      by ALES. Version 0.1.2 was reconciled for the first release, and the 0.1.3 corrective version is now consistent
      in `Cargo.toml`, `pyproject.toml`, `Cargo.lock`, version verification, release documentation, and tag
      expectations; the native stub has no literal package-version field to update.
  - [x] Prove that the final diff preserves checked hostile ranges, decompression/resource limits, panic containment,
    exact decoded-length checks, atomic extraction, symlink-safe paths, public JSON meanings, ABI3 behavior, and every
    open/closed finding in `AUDIT.md`, `SECURITY.md`, and `FUZZING.md`. Add deterministic regressions for any behavior
    changed during branch reconciliation. Baseline/candidate semantic output is byte-identical with canonical SHA-256
    `dbbf2bfc1d08ac4989ed6138273189ba08e15b07e70172610693d94619c0fd25`; new native and Python regressions cover
    on-demand chunk decoding, exact declared output, and preservation of an existing destination after decode failure.
  - [x] From the content-equivalent current default-branch tree, run every command in `AGENTS.md`: Ruff, mypy, Bandit,
    Python tests, both rustfmt
    checks, root/fuzz Clippy, Rust tests, fuzz builds, audit-exception and license checks, documentation, rustdoc, and
    version verification. Also require at least 90% Python and 70% native line coverage plus available cargo-audit and
    both pip-audit forms. All passed locally on 2026-09-21: 22 Python tests, 20 Rust tests on Rust 1.98.1 and 1.88.0,
    90% Python coverage, 80.03% native line coverage, five ASan fuzz campaigns, and no Cargo/pip audit findings.
  - [x] Rebuild production ABI3 wheel/sdist artifacts and rerun `scripts/benchmark.py --publish` from a clean commit so
    the optimized report names the exact candidate SHA. Confirm the large FAT speed/RSS/allocation improvements,
    unchanged logical output, bounded cancellation, and no material regression in the unchanged APFS paths. Final
    static-liblzma 0.1.2 wheel/sdist artifacts pass auditwheel, Twine, and an isolated install smoke test. The clean
    nine-sample report names candidate `27edd356536837c44ed9d309eb34c17355cbbd04`, records `published: true` and a
    clean worktree, retains a 274.9x FAT-listing speedup and 90.7% FAT-extraction reduction, and keeps unchanged APFS
    timings inside the baseline ranges; report SHA-256 is
    `3fc99a17452b874b78a4b9d78e6d888fd1fe40de0775c0cc4cf8ddd81be203c5`.
  - [x] Commit and push the reviewed corrective candidate, then require the complete hosted cross-platform, Rust 1.88
    MSRV, coverage, audit, sanitizer/fuzz, documentation, wheel, and tag-gated release workflows to pass. Fix a failing
    gate rather than weakening validation or editing retained audit history. The 0.1.2 candidate was reviewed in PR
    [`#3`](https://github.com/bwhitn/pydmg/pull/3); corrected CI run
    [`35608047824`](https://github.com/bwhitn/pydmg/actions/runs/35608047824) and fuzz run
    [`35608047919`](https://github.com/bwhitn/pydmg/actions/runs/35608047919) pass. PR #3 was merged as
    `d5808ff5e928819e2471c57175fa2323eb35f535`, and tagged release run
    [`35611077830`](https://github.com/bwhitn/pydmg/actions/runs/35611077830) published 0.1.2 successfully. Corrective
    PR [`#4`](https://github.com/bwhitn/pydmg/pull/4) passed complete CI run
    [`35617605668`](https://github.com/bwhitn/pydmg/actions/runs/35617605668) and all four PR fuzz jobs in run
    [`35617605544`](https://github.com/bwhitn/pydmg/actions/runs/35617605544), then merged as
    `36c6635369652f43a6f88211e905873e51bc1148`. Manual release preflight
    [`35618182500`](https://github.com/bwhitn/pydmg/actions/runs/35618182500) also passed before tagging.
  - [x] Publish the corrective immutable package release only after those gates pass. Tag `v0.1.3` names exact merge
    revision `36c6635369652f43a6f88211e905873e51bc1148`; trusted release run
    [`35619257967`](https://github.com/bwhitn/pydmg/actions/runs/35619257967) passed every quality, security, coverage,
    performance, fuzz, wheel, sdist, metadata, and publishing job. PyPI serves five non-yanked ABI3 wheels plus the
    sdist, and a fresh wheel-only install parsed the committed FAT fixture and the authorized Apple image. ALES now
    exact-pins `pydmg==0.1.3` with refreshed lock hashes and passed its DMG/Docker regressions, 1,959-test host and
    contained test-image suites, license/SBOM and stripped/pruned runtime checks, exact output/ObjectRules and stored
    artifact parity, and authorized-corpus performance acceptance.
