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

1. Choose the release version, for example `0.1.1`.
2. Ensure versions match in both files:
   - `pyproject.toml`: `project.version`
   - `Cargo.toml`: `package.version`
3. Run local quality gates:

```bash
. .venv/bin/activate
python scripts/verify_versions.py --expected 0.1.1
python scripts/check_licenses.py
python scripts/build_pydoc.py --cleanup
pytest -q
ruff check .
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

4. Build and validate distributables:

```bash
. .venv/bin/activate
rm -rf dist
maturin build --release --out dist
maturin sdist --out dist
twine check dist/*
```

5. Commit release changes.
6. Create and push the release tag:

```bash
git tag -a v0.1.1 -m "Release v0.1.1"
git push origin v0.1.1
```

7. Watch the `Release` workflow in GitHub Actions.
8. Confirm artifacts appear on PyPI and install test passes:

```bash
pip install pydmg==0.1.1
python -c "import pydmg; print(pydmg.__version__)"
```

## Notes

- The release workflow enforces version consistency and tag matching.
- `sdist` is included as the architecture-independent source distribution.
- You can run `workflow_dispatch` manually and provide `release_version` for preflight checks.
- License policy is validated via `scripts/check_licenses.py`.
