Third-party fixture sources:

- `hfsp-small.img.xz`
  - Source: `https://github.com/penguin359/hfsplus-rs`
  - Upstream path: `hfsp-small.img.xz`
  - Purpose: HFS+ positive parsing/listing/extraction regression test
  - Upstream license: MIT (see upstream `LICENSE`)
  - SHA-256: `2acdcd3fafb7974c3fb1820563aacfef412ae9cf4dfbdd5862b42f72b6237145`

- `fat32-large-sample.dmg`
  - Source: generated locally with `pydmg.create_dmg(...)`
  - Purpose: FAT positive parsing metadata fixture (`fat_type: fat32`)
  - Generation parameters: `volume_label='FAT32TST'`, `total_sectors=1048576`
  - SHA-256: `cdcff1aa80ee44f0efef3b7d5155ce3334fbfc684b3ab1e2ede2d33bb4dea427`

- `apfs-linearmouse-v0.10.2.dmg`
  - Source: `https://github.com/linearmouse/linearmouse/releases/download/v0.10.2/LinearMouse.dmg`
  - Upstream project: `linearmouse/linearmouse`
  - Upstream project license: MIT
  - Purpose: APFS positive parsing/listing/extraction regression test
  - SHA-256: `ed331d1597bdf93c7122c7120c08454e7ed3d254eb447dc4122f83a3355ef414`
