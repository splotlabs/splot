## Context

The ordinary coefficient branch is being staged from explicit caller facts toward
runtime AV2 section 5.20.7.27 `coeffs()` integration. The current
`Mode_To_Txfm` handoff covers only non-lossless intra chroma non-directional
`UVMode`; directional `UVMode` still returns
`UnsupportedModeToTxfmSubset { reason: "directional UVMode" }`.

AV2 section 5.20.7.29 defines the directional chroma path as
`pAngle = Mode_To_Angle[UVMode] + AngleDeltaUV * ANGLE_STEP`, then
`wide_angle_mapping(UVMode, Tx_Width[txSz], Tx_Height[txSz], pAngle)`, then
`Mode_To_Txfm[mode]`. `Mode_To_Angle`, `Mode_To_Txfm`, and transform-size
conversion tables are already generated in `splot-core::tables::conversion`.

## Goals / Non-Goals

**Goals:**

- Add a loaded-but-unwired decode-local handoff for intra chroma directional
  `UVMode`, tracked by
  `DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF`.
- Derive the mapped `mode` and resulting `PlaneTxType` before delegating to the
  existing transform-size/scan ordinary branch.
- Keep failures typed and fail-atomic before CDF, symbol, or coefficient-context
  mutation.
- Preserve existing behavior for all-zero, non-directional, chroma-DCT-only,
  luma/inter/lossless rejection, and transform-set fallback to `DCT_DCT`.

**Non-Goals:**

- Runtime `coeffs()` wiring.
- Luma/inter `TxTypes` lookup state.
- FSC/IDTX lossless branches.
- Parsing `AngleDeltaUV` from block syntax.
- Dequantization, inverse transform, residual add, output/reference refresh, or
  AVM/dav2d byte-match proof.

## Decisions

- Reuse the existing `Mode_To_Txfm` handoff rather than adding a new top-level
  wrapper. This keeps the directional branch in the exact function that already
  owns intra chroma transform-type derivation and leaves upstream `txSet` and
  lossless wrappers unchanged.
- Add `angle_delta_uv` to `CoeffOrdinaryBranchModeToTxfmBaseConfig`,
  `CoeffOrdinaryBranchTxSetBaseConfig`, and
  `CoeffOrdinaryBranchLosslessBaseConfig`. The value remains caller-resolved
  until broad block syntax is wired; zero preserves existing non-directional
  behavior.
- Implement `wide_angle_mapping` as a small decode-local helper cited to AV2
  section 5.20.7.29. The helper returns only the remapped mode because the unused
  angle is not consumed by `compute_tx_type`.
- Validate generated table domains before delegating. Invalid `UVMode` or table
  values fail before state mutation; malformed dimensions continue to use the
  existing transform-size table errors.

## Risks / Trade-offs

- [Risk] The caller-resolved `AngleDeltaUV` can be outside the future syntax
  domain. -> Mitigation: use checked signed arithmetic and keep runtime syntax
  parsing out of scope for this wrapper; future block syntax will constrain the
  value at its source.
- [Risk] The new config field touches several staged test builders. ->
  Mitigation: default it to zero in existing helpers and add targeted
  directional tests proving both no-remap and remap behavior.
- [Risk] This still does not make runtime decode output broader. -> Mitigation:
  track it as partial in the implementation and decoder support matrices and
  keep roadmap wording explicit.
