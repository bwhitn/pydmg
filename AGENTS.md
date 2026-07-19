# Repository Guide For Agents

## Scope and layout

- `src/lib.rs` is the Rust parser, decompressor, filesystem boundary, and PyO3 module.
- `python/pydmg/` is the typed public Python API; keep `_pydmg.pyi` synchronized with every native
  function signature.
- `tests/` contains Python integration and adversarial regressions.
- `fuzz/` and `scripts/prepare_fuzz_corpus.py` contain the structure-aware fuzz harness.
- `AUDIT.md`, `SECURITY.md`, and `FUZZING.md` are maintained evidence, not aspirational marketing.
  Update status and commands whenever behavior or gates change; never delete historical findings.

Only change source owned by this repository. If a defect is in a registry dependency, system
toolchain, hosted runner, or other external project, contain it at this repository's boundary when
possible and document the remaining upstream defect. Do not edit vendored registry or host files.

## Required verification

Run the smallest relevant checks while iterating, then run the complete local suite before handoff:

```bash
ruff check .
mypy
bandit -q -r python scripts
pytest -q
cargo fmt --all -- --check
cargo fmt --manifest-path fuzz/Cargo.toml -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo clippy --manifest-path fuzz/Cargo.toml --bins -- -D warnings
cargo test --locked --all-targets --all-features
cargo check --manifest-path fuzz/Cargo.toml --bins
python scripts/check_audit_exceptions.py
python scripts/check_licenses.py
python scripts/build_pydoc.py --cleanup
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
python scripts/verify_versions.py
```

Also run `cargo audit`, `python -m pip_audit .`, and `python -m pip_audit` when their external
databases/tools are available. The two pip-audit forms cover the project dependency set and the
active development-tool environment separately. Do not silently add advisory ignores: every
exception must be narrow, listed in both `.cargo/audit.toml` and
`security/audit-exceptions.json`, linked to an audit finding, and unexpired.

Python line coverage must remain at least 90%. Native line coverage must remain at least 70%. The
GitHub workflows are the canonical cross-platform, MSRV, coverage, audit, and sanitizer gates.

## Security invariants

Treat DMGs, raw filesystems, plist values, BLKX tables, compressed chunks, metadata, and filenames
as hostile input.

- Validate `offset + length` without overflow and against the actual input size before reading or
  allocating.
- Keep declared-count, nesting, expanded-byte, file-count, and aggregate-output limits effective.
- Bound decompression before materializing output. New compression formats require positive and
  over-limit tests plus fuzz coverage.
- Do not let dependency panics cross the Python API boundary. Keep upstream parser defects recorded
  even when locally contained.
- Extraction paths derived from an image must remain beneath a canonical output root and must not
  traverse symlinks. Write caller-selected outputs through a sibling temporary file and atomic
  rename so failure cannot corrupt the previous destination.
- CRC32 is only a corruption check; never describe it as authenticity or signature verification.

Every security fix needs a deterministic regression that demonstrates rejection or containment
without committing a sensitive proof of concept.

## Style and compatibility

- Preserve Python 3.9 compatibility and strict mypy cleanliness. Public JSON payloads are dynamic,
  so narrow `Any` at the `json.loads` boundary with an explicit `cast`.
- Keep Rust compatible with the `rust-version` in `Cargo.toml`, format with rustfmt, and treat all
  Clippy warnings as errors.
- Commit the root `Cargo.lock`; the fuzz crate lock remains ignored.
- Keep GitHub Actions pinned to reviewed full commit SHAs with a readable version comment.
- Preserve fixture provenance and SHA-256 records. Do not commit fuzz crashes or local corpora.
