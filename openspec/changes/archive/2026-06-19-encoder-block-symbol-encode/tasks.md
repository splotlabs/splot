## 1. Production entropy-coding entry point

- [x] 1.1 Add `encode_block_symbol_trace(trace) -> Result<Vec<u8>>` driving the §8.2 `SymbolEncoder` to coded bytes.
- [x] 1.2 Refactor `roundtrip_block_symbol_trace` to call it for the encode half (no behaviour change to the roundtrip).

## 2. Tests

- [x] 2.1 Prove the function emits non-empty bytes for the complete all-zero intra block that decode back to `[0,0,0,1,1,1]` (equal to the roundtrip's bytes).
- [x] 2.2 Prove the function is deterministic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-BLOCK-SYMBOL-ENCODE` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming a tile-group payload, OBU, frame, packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
