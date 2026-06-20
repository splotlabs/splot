## 1. Single-tile first-group structure constructor

- [x] 1.1 Add `TileGroupStructure::single_tile_first_group() -> TileGroupStructure` in `splot-core` (flag `false`, `tg_start 0`, `tg_end 0`, `outcome Complete`, `header_bytes`/`payload_size` `None`), preserving `#[non_exhaustive]`.

## 2. Tests

- [x] 2.1 Prove the constructor has the canonical single-tile fields.
- [x] 2.2 Prove `write_tile_group_structure` of the constructed structure reparses to the same `flag` / `tg_start` / `tg_end` syntax fields and `Complete` (semantic round-trip).

## 3. Tracking and verification

- [x] 3.1 Add `ENC-WRITER-INPUT-STRUCTURE` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming a tile-group OBU, frame-header / sequence-header constructor, frame, packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused core tests, feature-status checks, and `cargo xtask ci`.
