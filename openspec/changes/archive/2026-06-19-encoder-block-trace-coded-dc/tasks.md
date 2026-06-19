## 1. Coded DC block trace API

- [x] 1.1 Add `pub(crate)` `luma_dc_coded_tokens(coeff_cdf_q_ctx, magnitude, negative)` to `coefficient_tokenization` returning the four ordered coded luma DC tokens.
- [x] 1.2 Add an equivalence test asserting `luma_dc_coded_tokens` matches `tokenize_coefficients` for the supported magnitude/sign range.
- [x] 1.3 Extend `block_symbol_trace` with `compose_minimal_intra_dc_coded_block_trace` returning the mode prefix, coded luma residual, and all-zero U/V `txb_skip`.
- [x] 1.4 Route the `eob_pt_16`, `coeff_base_lf_eob`, and `dc_sign` CDF rows through the unified §8.2 roundtrip.
- [x] 1.5 Cite AV2 §5.20.5.3 (mode info), §5.20.7.27 (coded `residual()`), and §8.3.2 (coefficient CDF contexts).

## 2. Coded DC block trace tests

- [x] 2.1 Prove the coded trace is the mode prefix, coded luma residual (txb_skip=0, eob_pt_16, coeff_base_eob, dc_sign), then all-zero U/V txb_skip, in order.
- [x] 2.2 Prove the nine-symbol sequence roundtrips through one §8.2 symbol encoder/decoder back to the same ordered symbols with shared CDF state.
- [x] 2.3 Prove the roundtrip is deterministic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-BLOCK-TRACE-CODED-DC` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming multi-coefficient blocks, coeff base-range, chroma coefficients, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
