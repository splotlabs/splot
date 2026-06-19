## 1. Unified block-symbol trace API

- [x] 1.1 Add a `pub(crate)` luma all-zero (`txb_skip`) token accessor to `coefficient_tokenization`.
- [x] 1.2 Extend `block_symbol_trace` with a unified `BlockSymbolToken` spanning the intra-mode and coefficient token kinds, and a `compose_minimal_intra_dc_all_zero_block_trace` function returning the ordered `y_mode_set`, `y_mode_index`, `uv_mode`, luma `txb_skip` sequence.
- [x] 1.3 Add a unified §8.2 roundtrip holding the mode and `txb_skip` CDF rows from `splot-core` defaults, routing each token to its scoped row, with typed errors keyed by token index.
- [x] 1.4 Cite AV2 §5.20.5.3 (mode info before `residual()`) and §5.20.7.27 (`all_zero`).

## 2. Unified block-symbol trace tests

- [x] 2.1 Prove the composed trace is the ordered mode prefix followed by the luma `txb_skip` all-zero token.
- [x] 2.2 Prove the composed sequence roundtrips through one §8.2 symbol encoder/decoder back to the same ordered symbols with shared CDF state.
- [x] 2.3 Prove the roundtrip is deterministic; add a negative test for an unsupported selector in the unified CDF router.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-LUMA-SKIP` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming chroma all-zero, full coefficients, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
