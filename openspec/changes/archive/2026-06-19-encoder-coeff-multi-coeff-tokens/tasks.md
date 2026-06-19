## 1. Multi-coefficient token accessors

- [x] 1.1 Add a `coefficient_tokenization/multi_coeff.rs` submodule with `coded_luma_all_zero_token`, `eob_pt_16_token(coeff_cdf_q_ctx, eob_ctx, symbol)`, and `coeff_base_lf_eob_token(coeff_cdf_q_ctx, ctx, level)` (symbol = level − 1).
- [x] 1.2 Wire the eob = 2 AC `coeff_base_eob` context (1) row into the generic `CoefficientTokenCdfRows` router (`coeff_base_eob_ctx(c=1) = SIG_COEF_CONTEXTS_EOB − 3 = 1`).

## 2. Tests

- [x] 2.1 Prove each accessor carries the expected symbol (coded all_zero = 0; eob_pt_16 = 1 → eob 2; coeff_base_eob level 1 → symbol 0 at ctx 1).
- [x] 2.2 Prove the eob = 2 CDF subsequence (coded all_zero, eob_pt_16 = 1, AC coeff_base_eob ctx 1, DC coeff_base ctx 1) roundtrips through the generic § 8.2 router to `[0, 1, 0, 0]`.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-COEFF-MULTI-TOKENS` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming a multi-coefficient trace, chroma/high-frequency contexts, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
