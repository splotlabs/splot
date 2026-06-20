## 1. IVF temporal-unit assembler

- [x] 1.1 Add `encode_minimal_intra_clk_ivf(tile_data)`: concatenate the TD, sequence-header, and frame Annex B OBUs into one AV2 temporal unit (decoder order) and wrap it in one AV02 64x64 IVF frame; add the `MinimalIntraIvfError` typed error; re-export from `frame`.

## 2. Tests

- [x] 2.1 A consistency test: the result is a valid AV02 64x64 IVF with one frame whose Annex B payload is `[OBU_TEMPORAL_DELIMITER, OBU_SEQUENCE_HEADER, OBU_CLOSED_LOOP_KEY]`, and `from_sequence` of the sequence header is the frozen 64x64 single-picture `Block64x64` tier the frame header was built against.
- [x] 2.2 A reject test: empty `tile_data` propagates the typed `Frame` error, no panic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-MINIMAL-INTRA-IVF` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Keep tracking honest: a structurally valid IVF with consistent headers, NOT yet a decode-hash match to the conformance vector (the coded tile content is a caller input); no claim of a complete coded tile, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused core tests, feature-status checks, and `cargo xtask ci`.
