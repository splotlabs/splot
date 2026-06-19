## Why

Multi-coefficient blocks (eob > 1) read a non-EOB `coeff_base` symbol for the
lower-scan coefficients, coded with the `TileCoeffBaseLfCdf` row for low-frequency
luma. The single-DC bricks only have the EOB-position `coeff_base_eob` token, so
this change adds the non-EOB `coeff_base` low-frequency luma token and its CDF row
routing — the second building block (after `ENC-COEFF-BASE-LF-CONTEXT`) the eob > 1
trace brick needs.

## What Changes

- Add `ENC-COEFF-BASE-LF-TOKEN` as a private `splot-encode` encoder-tool feature.
- Add the `CoefficientTokenSyntax::CoeffBase` syntax (the non-EOB base level) and
  the `CoefficientCdfRowSelector::CoeffBaseLf { coeff_cdf_q_ctx, tx_size, ctx,
  tcq_ctx }` selector (`TileCoeffBaseLfCdf[q][tx_size][ctx][tcq_ctx]`).
- Add `pub(crate)` `coeff_base_lf_token(coeff_cdf_q_ctx, ctx, tcq_ctx, level)`
  (the non-EOB base level equals the decoded symbol).
- Wire the `coeff_base_lf` row into the generic `CoefficientTokenCdfRows` roundtrip
  router at the eob=2 DC context (1, the `coeff_base_lf_luma_context` result for an
  AC level-1 neighbour) and the TCQ-off context (0), so the token roundtrips through
  the in-tree AV2 § 8.2 coder.
- Add the `CoeffBase` arm to the closed-loop single-DC recovery helper (a no-op,
  documented — the single-DC path never carries a non-EOB `coeff_base`).
- The token is available but not yet composed into a trace (the eob > 1 trace brick
  does that). No packet output.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the non-EOB `coeff_base` low-frequency
  luma token and its CDF row.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization.rs` (+ sibling
  tests), `crates/splot-encode/src/closed_loop.rs` (exhaustive-match arm).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; crate-private, not re-exported.
- Dependency impact: none new; imports the existing `splot-core`
  `DEFAULT_COEFF_BASE_LF_CDF` table.
- Validator/CLI impact: none.
