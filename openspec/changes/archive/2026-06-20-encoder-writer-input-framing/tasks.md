## 1. Single-tile framing constructor

- [x] 1.1 Add `TileGroupFraming::single_tile(tile_size) -> TileGroupFraming` in `splot-core` (`TileNum 0`, no size field, coded region from offset 0, defect None), preserving `#[non_exhaustive]`.

## 2. Tests

- [x] 2.1 Prove the constructor is value-equal to `parse_tile_group_framing(region, 0, 0, _, false)` for a single-tile region.
- [x] 2.2 Prove a `write_tile_group_payload` of the constructed framing produces the coded bytes and reparses value-equal to the constructed framing.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-WRITER-INPUT-FRAMING` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming a tile-group OBU, frame header constructor, frame, packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused core tests, feature-status checks, and `cargo xtask ci`.
