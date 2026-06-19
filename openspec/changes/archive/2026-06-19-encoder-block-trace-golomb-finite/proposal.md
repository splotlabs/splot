## Why

The coded luma DC trace caps at magnitude 7 — magnitude 8 reaches the §5.20.7.27
`maxLevel`, at which §5.20.7.28 `read_quant` emits the golomb (`coeff_rem`) tail.
Now that the §8.2.5 bypass-literal token exists, this change models the golomb
finite-q path, extending the coded luma DC range to magnitude 8..17 — the
golomb tail's first real consumer of the bypass token.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE` as a private `splot-encode`
  encoder-tool feature.
- Add `pub(crate)` `luma_dc_golomb_level_tokens(coeff_cdf_q_ctx)` (the fixed
  `all_zero=0` / `eob_pt_16=0` / `coeff_base_eob=LF_NUM_BASE_LEVELS` /
  `coeff_br=COEFF_BASE_RANGE` level tokens that drive the level to `maxLevel`) and
  factor out `luma_dc_sign_token` in `coefficient_tokenization`.
- Extend `block_symbol_trace` with `compose_minimal_intra_dc_golomb_block_trace`:
  the mode prefix, the golomb level tokens, the luma `dc_sign` CDF token, then the
  §5.20.7.28 finite-q golomb `coeff_rem` bypass bits encoding
  `x = magnitude - maxLevel` (`m = 1`, `q = x >> 1`, `coeff_rem = x & 1`) — the
  §5.20.7.27 sign+quant pass reads the sign before calling `read_quant` — then
  all-zero U/V `txb_skip`.
- Prove the trace roundtrips through one §8.2 coder and that the decoded golomb
  bits reconstruct the encoded magnitude via the decoder's `read_quant` finite-q
  arithmetic.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the §5.20.7.28 finite-q golomb tail of a
  coded luma DC coefficient.

## Impact

- Affected code: `crates/splot-encode` internals and tests (`coefficient_tokenization`,
  `block_symbol_trace`).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; reuses the existing `splot-core` symbol coder
  primitives.
- Validator/CLI impact: none; no coded packets or public encoder success path.
