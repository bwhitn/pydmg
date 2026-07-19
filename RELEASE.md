# Release Checklist

Use this checklist for every PyPI release.

## One-time setup

1. Create the `pydmg` project on PyPI.
2. Configure trusted publishing for this repository and workflow:
   - Workflow file: `.github/workflows/release.yml`
   - Environment: `pypi`
3. Set repository metadata once your GitHub remote is finalized:
   - `pyproject.toml` (`[project.urls]`)
   - `Cargo.toml` (`repository`, optionally `homepage` and `documentation`)

## Per-release steps

1. Choose the release version, for example `0.1.0`.
2. Ensure versions match in both files:
   - `pyproject.toml`: `project.version`
   - `Cargo.toml`: `package.version`
3. Run local quality gates:

```bash
. .venv/bin/activate
python scripts/verify_versions.py --expected 0.1.0
python scripts/check_audit_exceptions.py
python scripts/check_licenses.py
python scripts/build_pydoc.py --cleanup
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
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
coverage run -m pytest -q
coverage report --fail-under=90
```

Generate Python and native Rust coverage reports, and investigate any regression from the baseline
in [`AUDIT.md`](AUDIT.md). Run all bounded AddressSanitizer fuzz campaigns in
[`FUZZING.md`](FUZZING.md); parser or dependency changes require a longer campaign. Do not release
with an unexplained advisory, sanitizer finding, crash, hang, or uncontrolled allocation.

4. Build and validate distributables:

```bash
. .venv/bin/activate
rm -rf dist
LZMA_API_STATIC=1 maturin build --release --locked --compatibility pypi --auditwheel check --out dist
maturin sdist --out dist
twine check dist/*
```

5. Commit release changes.
6. Create and push the release tag:

```bash
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0
```

7. Watch the `Release` workflow in GitHub Actions.
8. Confirm artifacts appear on PyPI and install test passes:

```bash
pip install pydmg==0.1.0
python -c "import pydmg; print(pydmg.__version__)"
```

## Notes

- The release workflow enforces version consistency, lint, strict typing, tests, advisory and
  license policy, Python/native coverage floors, documentation, and direct-parser fuzz smoke before
  it builds or publishes artifacts.
- `sdist` is included as the architecture-independent source distribution.
- You can run `workflow_dispatch` manually and provide `release_version` for preflight checks.
- License policy is validated via `scripts/check_licenses.py`.
- Rust advisory exceptions are exact and expire; validate
  `security/audit-exceptions.json` rather than extending an exception silently.
