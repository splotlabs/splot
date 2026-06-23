## 1. Skip tile_data emission

- [x] 1.1 Add `encode_general_intra_dc_skip_tile_data()`: compose the brick-2 skip trace and finalize it via `encode_block_symbol_trace` (§ 8.2.4), returning the `tile_data` bytes.
- [x] 1.2 Document the muxing contract (`base_q_idx <= 90` → q-context 0, `disable_cdf_update == 0` → `CdfUpdateMode::Enabled`) so a later brick assembles a decodable frame.

## 2. Tests

- [x] 2.1 The bytes are non-empty (a zero-size tile is a § 8.2.2 defect).
- [x] 2.2 The bytes equal the proven trace's roundtrip bytes, and that trace decodes to `[0, 0, 0, 0, 1, 1, 1]` — so the standalone emission inherits the decodability proof.
- [x] 2.3 Emission is deterministic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-GENERAL-INTRA-SKIP-TILE-DATA` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Keep tracking honest: one in-memory `tile_data` byte producer, not a tile-group OBU, a frame, a packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1; the cross-crate decode oracle is a later brick.
- [x] 3.3 Run OpenSpec validation, focused encode tests, feature-status checks, and `cargo xtask ci`.
