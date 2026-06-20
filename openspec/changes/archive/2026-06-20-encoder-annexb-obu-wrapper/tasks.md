## 1. Annex B OBU wrapper

- [x] 1.1 Add `encode_minimal_intra_clk_annexb_obu(tile_data: &[u8]) -> Result<Vec<u8>, MinimalIntraTileGroupError>`: build the brick-4 payload, the no-extension `OBU_CLOSED_LOOP_KEY` § 5.2.2 header (inferred layer ids 0), and drive `write_annexb_obu`; re-export it from `frame`.

## 2. Tests

- [x] 2.1 A round-trip test: `parse_annex_b_obus_partial` of the result is exactly one `OBU_CLOSED_LOOP_KEY` whose payload equals the brick-4 `tile_group_obu()` payload (ending in the coded tile bytes).
- [x] 2.2 A reject test: empty `tile_data` propagates the typed zero-size-tile `Write` error, no panic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-MINIMAL-INTRA-CLK-ANNEXB-OBU` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Keep tracking honest: a single frame OBU, not a decodable temporal unit; no claim of a temporal delimiter, a sequence header, an IVF stream, a complete coded tile, a packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused core tests, feature-status checks, and `cargo xtask ci`.
