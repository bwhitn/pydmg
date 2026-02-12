Third-party fixture sources:

- `hfsp-small.img.xz`
  - Source: `https://github.com/penguin359/hfsplus-rs`
  - Upstream path: `hfsp-small.img.xz`
  - Purpose: HFS+ positive parsing/listing/extraction regression test
  - Upstream license: MIT (see upstream `LICENSE`)

- `fat32-large-sample.dmg`
  - Source: generated locally with `pydmg.create_dmg(...)`
  - Purpose: FAT positive parsing metadata fixture (`fat_type: fat32`)
  - Generation parameters: `volume_label='FAT32TST'`, `total_sectors=1048576`

- `apfs-linearmouse-v0.10.2.dmg`
  - Source: `https://github.com/linearmouse/linearmouse/releases/download/v0.10.2/LinearMouse.dmg`
  - Upstream project: `linearmouse/linearmouse`
  - Upstream project license: MIT
  - Purpose: APFS positive parsing/listing/extraction regression test
