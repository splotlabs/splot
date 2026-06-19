## Why

The ordinary coefficient branch still rejects directional chroma `UVMode`
inside the staged AV2 section 5.20.7.29 `compute_tx_type` handoff. Supporting
that subset removes one more caller-resolved transform-type gap before runtime
`coeffs()` wiring.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF`.
- Extend the crate-private ordinary coefficient `Mode_To_Txfm` handoff to derive
  directional chroma `PlaneTxType` from `Mode_To_Angle[UVMode]`,
  caller-resolved `AngleDeltaUV`, `ANGLE_STEP`, `wide_angle_mapping`, and
  generated transform-size dimensions.
- Preserve the existing non-directional, `enable_chroma_dctonly`, all-zero, and
  fail-atomic unsupported-subset behavior.
- Update implementation matrix, decoder support/conformance coverage, roadmap,
  and generated status docs.

## Capabilities

### New Capabilities

- `coeff-ordinary-branch-directional-uv-handoff`: loaded-but-unwired ordinary
  coefficient branch support for the AV2 section 5.20.7.29 intra chroma
  directional `UVMode` transform-type subset.

### Modified Capabilities

- `decoder-support`: record the directional UV ordinary branch row and its
  remaining decoder gaps.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and `xtask/src/decoder_conformance_coverage.rs`.
- No new dependencies, public API, crate dependency graph, encoder, writer, or
  runtime decode-output changes.
- Non-goals: runtime `coeffs()` integration, luma/inter/`TxTypes` lookup,
  FSC/IDTX lossless handling, block syntax traversal, dequantization, inverse
  transform, residual add, output/reference refresh, and AVM/dav2d byte-match
  proof.
