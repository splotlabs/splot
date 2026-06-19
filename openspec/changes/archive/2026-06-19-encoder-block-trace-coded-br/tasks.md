## 1. Base-range coefficient tokenization

- [x] 1.1 Add the `coeff_br` token syntax and a `CoeffBrLf` (`TileCoeffBrLfCdf`) CDF-row selector to `coefficient_tokenization`.
- [x] 1.2 Make `luma_dc_coded_tokens` the single coded-DC token source (variable length) and have it emit `coeff_br` when `magnitude > LF_NUM_BASE_LEVELS`, saturating `coeff_base_eob` at level 5.
- [x] 1.3 Delegate `tokenize_coefficients` to `luma_dc_coded_tokens` and raise the supported magnitude cap to `LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE = 7` (magnitude 8 reaches maxLevel → golomb, rejected).
- [x] 1.4 Add the `coeff_br_lf` CDF row to the tokenizer roundtrip rows and route the `CoeffBrLf` selector.
- [x] 1.5 Cite AV2 §5.20.7.27 (`coeff_br`, `level += coeff_br`) and §8.3.2 (low-frequency `coeff_br` CDF + DC ctx 0); mirror the decoder's `BrLf`/`CoeffBrContext` derivation.

## 2. Base-range block trace and tests

- [x] 2.1 Extend `block_symbol_trace` with `compose_minimal_intra_dc_br_block_trace` and route the `coeff_br_lf` CDF row.
- [x] 2.2 Extend `coded_dc_tokens_match_tokenizer` to the full 1..=7 range; add base-range token-shape and roundtrip tests.
- [x] 2.3 Prove the ten-symbol base-range trace roundtrips deterministically through one §8.2 coder.
- [x] 2.4 Update the `closed_loop` DC-recovery test helper to accumulate `coeff_br` into the level.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-CODED-BR` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming the golomb tail, multi-coefficient blocks, chroma coefficients, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
