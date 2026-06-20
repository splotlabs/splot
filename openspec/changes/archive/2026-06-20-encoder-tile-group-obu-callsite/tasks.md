## 1. Tile-group OBU payload assembler

- [x] 1.1 Add `encode_minimal_intra_clk_tile_group_obu(tile_data: &[u8]) -> Result<Vec<u8>, MinimalIntraTileGroupError>`: build `(core, seq)` via `build_minimal_intra_clk_core`, single-tile structure + framing over `tile_data`, drive `write_tile_group_obu`, return the payload bytes.
- [x] 1.2 Add the `MinimalIntraTileGroupError` typed error (`Core(#[from] MinimalIntraCoreError)` / `Write(#[from] WriteError)`) and re-export both from `frame`.

## 2. Tests

- [x] 2.1 A round-trip test: encode coded tile bytes, reparse the payload's tile-group prefix (`is_first_tile_group`, `frame_header_present_flag`, embedded frame header), and assert the tile data is the trailing region of the payload.
- [x] 2.2 A reject test: empty `tile_data` yields a typed `Write` error (the § 8.2.2 zero-size-tile defect), no panic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-MINIMAL-INTRA-TILE-GROUP-OBU` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Keep tracking honest: no claim of an OBU header/size wrapper, a complete coded tile, a frame, a packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused core tests, feature-status checks, and `cargo xtask ci`.
