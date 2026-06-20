## Why

The general intra decode path decodes the § 5.20.5.3 block mode info and then
stops at the residual step. The next step toward decoding a real AVM-generated
minimal-tool intra frame — and the first genuine AV2 § 5.20.7.27 `coeffs()`
decode through the coefficient-loop machinery on a real bitstream — is to decode
the single luma transform block's coefficients.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-LUMA-COEFFS`.
- Add `decode_general_intra_luma_coeffs` that decodes the AV2 § 5.20.7.27
  `coeffs()` syntax for the single non-partitioned 64x64 luma transform block:
  it reads the `all_zero` (`txb_skip`) symbol with the spec-derived § 8.3.2
  context (`coeff_cdf_q_ctx` from `base_q_idx`, `txSzCtx` from the generated
  § 9.2 `Tx_Size_Sqr` / `Tx_Size_Sqr_Up` tables, and the first-block
  tx-fills-block luma context), and when `all_zero == 0` routes the nonzero
  coefficient pass through `apply_coeff_use_fsc_branch_from_frame_facts`
  (`PlaneTxType == DCT_DCT`, plane 0, intra), returning the decoded `Quant[]`
  and end-of-block.
- Wire it into the general intra frame path after mode decode, advancing the
  structured rejection from the residual step
  (`general_intra_residual_decode_unimplemented`) to chroma decode
  (`general_intra_chroma_decode_unimplemented`).
- Keep the frozen `base_q_idx == 255` minimal hash contract byte-identical.
- Add unit tests for the `txb_skip` transform-size context derivation and update
  the CLI test to assert the general intra fixture now decodes its luma
  coefficients and reaches the chroma step.
- Update decoder tracking, roadmap, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-luma-coeffs`: Crate-private AV2 § 5.20.7.27 luma
  transform-block coefficient decode for the general intra path.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra luma coefficient decode.

## Impact

- Affects `crates/splot-decode/src/tile_payload/general_intra_residual.rs` (new),
  `crates/splot-decode/src/tile_payload.rs`,
  `crates/splot-decode/src/runtime_minimal.rs`, and
  `crates/splot-cli/tests/decode_cli.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, `docs/DECODER-ROADMAP.md`, and
  generated status/coverage docs.
- No public API, dependency graph, encoder, validator, chroma coefficient
  decode, dequantization, inverse transform, residual add, reconstruction,
  output, reference-refresh, or in-repo AVM/dav2d integration changes are in
  scope. The decoded `Quant[]` is returned to the caller, not yet reconstructed.
