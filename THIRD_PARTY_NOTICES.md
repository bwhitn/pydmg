# Third-Party Notices

`pydmg` is licensed under the MIT License (see `LICENSE`).

This repository includes and links to third-party components. The project
remains MIT-licensed, and these components are used under their own licenses.

## Direct Rust Dependencies Used By `pydmg`

- `anyhow`, `base64`, `bzip2`, `crc32fast`, `flate2`, `pyo3 0.29`, `serde`,
  `serde_json`, and `tempfile`: `MIT OR Apache-2.0`
- `apple-dmg 0.5.0`: `Apache-2.0 OR MIT`
- `apfs 0.2.4`, `fatfs 0.3.6`, `gpt 4.1`, `hfsplus 0.2.4`, `lzfse 0.2`, `plist
  1.x`, and `udif 0.3.4`: `MIT`

Other direct/transitive Rust dependencies and SPDX expressions are recorded in
`Cargo.lock`.

## Test Fixture Sources

- `tests/fixtures/hfsp-small.img.xz`
  - Source: `https://github.com/penguin359/hfsplus-rs`
  - Upstream license: MIT
- `tests/fixtures/apfs-linearmouse-v0.10.2.dmg`
  - Source:
    `https://github.com/linearmouse/linearmouse/releases/download/v0.10.2/LinearMouse.dmg`
  - Upstream project license: MIT
- `tests/fixtures/fat32-large-sample.dmg`
  - Generated locally by this project for tests.

## License Policy Enforcement

CI runs `scripts/check_licenses.py` to verify Rust dependencies resolve to an
allowed permissive SPDX path.

No Paragon APFS SDK code is included in this repository.
