## Why

The coded block traces code only luma coefficients; chroma planes are always
all-zero. A real intra frame codes chroma coefficients. A first attempt (closed
PR #319) modeled the chroma DC as CDF-only, but review found the chroma DC sign is
a `sign_bit L(1)` *bypass literal* (§5.20.7.27 codes the `dc_sign` CDF only for the
luma DC and `dc_sign_horz_vert` for the directional luma axis signs), and that the
V `txb_skip` context gains `+6` once U is coded (`EobU != 0`). This change adds the
coded chroma U DC correctly, on top of the merged §8.2.5 bypass-literal token.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC` as a private `splot-encode`
  encoder-tool feature.
- Add a `CoeffBaseLfEobUv` (`TileCoeffBaseLfEobUvCdf`) CDF-row selector and a
  `pub(crate)` `chroma_u_dc_coded_coeff_tokens(coeff_cdf_q_ctx, magnitude)`
  accessor to `coefficient_tokenization` returning the three coded chroma U DC
  *CDF* tokens (`txb_skip=0`, chroma `eob_pt_16`, chroma `coeff_base_eob`); it
  rejects magnitudes outside the base tier via a typed error.
- Extend `block_symbol_trace` with `compose_minimal_intra_dc_coded_chroma_block_trace`
  (coded luma + coded U CDF tokens + the U DC `sign_bit` as a `Bypass` literal +
  the all-zero V `txb_skip` at the §8.3.2 EobU context 6) and route the chroma
  `eob_pt_16`, `coeff_base_lf_eob_uv`, and V-EobU `txb_skip` CDF rows.
- Prove the twelve-token coded-chroma-block trace writes through one `SymbolEncoder`
  and decodes back through one `SymbolDecoder` with shared CDF state.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the coded chroma (U-plane) intra DC block
  symbol trace with the chroma `sign_bit` bypass literal and the V EobU context.

## Impact

- Affected code: `crates/splot-encode` internals and tests (`coefficient_tokenization`,
  `block_symbol_trace`, one `error` variant).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; reuses the existing `splot-core` symbol coder and CDF
  tables.
- Validator/CLI impact: none; no coded packets or public encoder success path.
