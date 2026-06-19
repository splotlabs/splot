## 1. coeff_base_lf token + CDF row

- [x] 1.1 Add `CoefficientTokenSyntax::CoeffBase` and `CoefficientCdfRowSelector::CoeffBaseLf { coeff_cdf_q_ctx, tx_size, ctx, tcq_ctx }` (`TileCoeffBaseLfCdf[q][tx_size][ctx][tcq_ctx]`).
- [x] 1.2 Add `pub(crate)` `coeff_base_lf_token(coeff_cdf_q_ctx, ctx, tcq_ctx, level)` (non-EOB base level == symbol).
- [x] 1.3 Wire the `coeff_base_lf` row into `CoefficientTokenCdfRows` at the eob=2 DC context (1) and TCQ-off context (0), and add the `CoeffBase` arm to the closed-loop single-DC recovery helper (no-op, documented).

## 2. Tests

- [x] 2.1 Prove the token carries the non-EOB base level (symbol == level) and selects the `CoeffBaseLf` row at ctx 1 / tcq_ctx 0.
- [x] 2.2 Prove the token roundtrips through the generic `roundtrip_entropy_tokens` helper for several levels.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-COEFF-BASE-LF-TOKEN` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming a multi-coefficient trace, chroma/parity-hidden contexts, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
