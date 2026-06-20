## Why

Completes the per-plane coded-residual set. The luma and chroma-U coded frames proved residual
on those planes; this proves it on V — the U block stays skipped, so V's `txb_skip` exercises
the dedicated `TileVTxbSkipCdf` path at the neutral (`EobU == 0`) context, and the residual
lands on V only.

## What Changes

- Add `ENC-GENERAL-INTRA-CODED-CHROMA-V-DC` as an encoder feature (splot-encode + splot-cli
  oracle).
- Add `general_intra_32x32_chroma_v_dc_coded_tokens(q, magnitude)`: the coded V DC tokens
  (`VTxbSkip == 0` at the neutral context, `eob_pt_1024` at chroma `eob_ctx 2`,
  `coeff_base_eob`) — reusing the chroma `eob_pt_1024` and `coeff_base_lf_eob_uv` rows (no new
  CDF rows).
- Add `compose_general_intra_coded_chroma_v_block_trace` (luma skip + U skip + V coded + V DC
  `sign_bit` § 8.2.5 bypass) and `splot_encode::emit_minimal_intra_coded_chroma_v_ivf()`.
- Add the cross-crate oracle: `splot decode` reconstructs flat luma 128, flat U 128, flat
  V 127.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the decodable coded-V intra frame.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/general_coded.rs` (+
  re-export), `crates/splot-encode/src/general_intra_trace.rs` (composer + emit),
  `crates/splot-encode/src/lib.rs`, `crates/splot-cli/tests/encode_decode_roundtrip.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage,
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added `splot-encode` function. No new CDF rows, no dependency-graph
  change.
