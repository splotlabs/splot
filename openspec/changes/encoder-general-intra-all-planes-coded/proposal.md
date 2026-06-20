## Why

The per-plane coded frames each code one plane in isolation. This frame codes **all three**
planes at once (luma + U + V), mirroring the AVM-validated q80 fixture's structure and
exercising the plane interaction: with the U plane coded (`EobU != 0`), the V `txb_skip` moves
to the § 8.3.2 context `6`. It is a step toward eventual byte-exact q80 reproduction.

## What Changes

- Add `ENC-GENERAL-INTRA-ALL-PLANES-CODED` as an encoder feature (splot-encode + splot-cli
  oracle).
- Parameterize `general_intra_32x32_chroma_v_dc_coded_tokens` by the V `txb_skip` context (the
  V-only caller passes the neutral `0`; the all-planes caller passes the `EobU != 0` context
  `6`).
- Add `compose_general_intra_all_planes_coded_block_trace` (coded luma + coded U + U sign +
  coded V at context 6 + V sign) and `splot_encode::emit_minimal_intra_all_planes_coded_ivf()`.
  Reuses all existing coded-DC tokens and CDF rows.
- Add the cross-crate oracle: `splot decode` reconstructs every plane flat at 127.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the all-planes-coded intra frame.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/general_coded.rs` (V ctx
  parameter), `crates/splot-encode/src/general_intra_trace.rs` (composer + emit + the V-only
  caller), `crates/splot-encode/src/lib.rs`,
  `crates/splot-cli/tests/encode_decode_roundtrip.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage,
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added `splot-encode` function. No new CDF rows, no dependency-graph
  change.
