## Why

The block-symbol trace currently composes only the all-zero block (every plane's
`txb_skip == 1`, no coefficients). A real intra frame codes coefficients. This
change adds the minimal *coded* intra block: a single luma DC coefficient whose
`residual()` emits `txb_skip == 0` then the `eob_pt_16`, `coeff_base_eob`, and
`dc_sign` symbols, proving the coded coefficient path roundtrips through one §8.2
coder alongside the mode prefix and all-zero chroma.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-CODED-DC` as a private `splot-encode` encoder-tool
  feature.
- Add a `pub(crate)` `luma_dc_coded_tokens(coeff_cdf_q_ctx, magnitude, negative)`
  accessor to `coefficient_tokenization` returning the four ordered coded luma
  DC-only tokens (`all_zero=0`, `eob_pt_16`, `coeff_base_eob`, `dc_sign`), proven
  by an equivalence test to mirror `tokenize_coefficients` exactly.
- Extend `block_symbol_trace` with `compose_minimal_intra_dc_coded_block_trace`,
  the ordered `y_mode_set`, `y_mode_index`, `uv_mode`, then the coded luma
  residual (`txb_skip=0`, `eob_pt_16`, `coeff_base_eob`, `dc_sign`), then the
  all-zero U and V `txb_skip`, and route the `eob_pt_16`, `coeff_base_lf_eob`, and
  `dc_sign` CDF rows through the unified §8.2 roundtrip.
- Prove the complete nine-symbol coded-block trace writes through one
  `SymbolEncoder` and decodes back through one `SymbolDecoder` with shared CDF
  state.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the minimal coded (non-all-zero) intra
  block symbol trace.

## Impact

- Affected code: `crates/splot-encode` internals and tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; reuses the existing `splot-core` symbol coder and CDF
  tables.
- Validator/CLI impact: none; no coded packets or public encoder success path.
