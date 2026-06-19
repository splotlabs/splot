## 1. Complete all-zero block trace API

- [x] 1.1 Add `pub(crate)` chroma U and V `all_zero` token accessors to `coefficient_tokenization`, with a new `VTxbSkip` CDF-row selector for the dedicated `TileVTxbSkipCdf`.
- [x] 1.2 Extend `block_symbol_trace` with `compose_minimal_intra_dc_complete_all_zero_block_trace` returning the ordered mode prefix then per-plane luma/U/V `txb_skip` tokens.
- [x] 1.3 Route the U `txb_skip` (`DEFAULT_TXB_SKIP_CDF[..][1][..][6]`) and V `txb_skip` (`DEFAULT_V_TXB_SKIP_CDF[..][0]`) rows through the unified §8.2 roundtrip.
- [x] 1.4 Cite AV2 §5.20.7.27 (per-plane `all_zero` in `residual()` order) and §8.3.2 (U +6 context; dedicated V `txb_skip` CDF).

## 2. Complete all-zero block trace tests

- [x] 2.1 Prove the complete trace is the mode prefix followed by luma, U, V `txb_skip` all-zero tokens in order.
- [x] 2.2 Prove the six-symbol sequence roundtrips through one §8.2 symbol encoder/decoder back to the same ordered symbols with shared CDF state.
- [x] 2.3 Prove the roundtrip is deterministic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming non-all-zero coefficients, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
