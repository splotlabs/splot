## Why

The general intra decode reconstructs two of the three § 7.13.2.8 MIDDLE
directional angles (`90 < pAngle < 180`): `D135_PRED` (pAngle 135) and
`D157_PRED` (pAngle 157), the latter at a row>0 corner via
`DECODE-GENERAL-INTRA-DIRECTIONAL-CORNER`. The remaining middle angle,
`D113_PRED` (pAngle 113, canonical § 9.2 mode 5), is still rejected. D113 is
VERTICAL-LEANING — `dx = Dr_Intra_Derivative[180 - 113] =
Dr_Intra_Derivative[67] = 24` and `dy = Dr_Intra_Derivative[113 - 90] =
Dr_Intra_Derivative[23] = 170` (§ 9.2) — so unlike D135 (`shift == 0`, the IDIF
reduces to a copy) and complementary to D157 (mostly left-branch), it reads
MOSTLY the above row + corner along the 113-degree projection with a NONZERO
`shift`, genuinely interpolating via the § 7.13.2.8 luma IDIF 4-tap. The two
prerequisites already exist: the § 7.13.2.8 luma IDIF 4-tap kernel
(`predict_intra_middle_directional_angle_rect_idif_into`,
`DECODE-GENERAL-INTRA-IDIF-D157`) and the real § 7.13.2.1 corner builder
(`build_directional_middle_edges` `(true, true)` arm reading
`CurrFrame[plane][y-1][x-1]`, `DECODE-GENERAL-INTRA-DIRECTIONAL-CORNER`). This is
a wiring + fixture brick that completes the middle-angle zone.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-ANGLE-D113`.
- Extend `IntraYMode::supported_directional` to map mode value 5 to
  `SupportedDirectionalLumaMode::D113`, and `middle_directional_angle` to map it
  to `IntraMiddleDirectionalAngle::D113` (the recon kernel's existing pAngle 113,
  `dx = Dr[67] = 24`, `dy = Dr[23] = 170`).
- Add `SupportedChromaMode::D113Follow` (resolved when `uv_mode == 0` over the
  D113 luma makes § 5.20.5.3 return `UVMode == D113_PRED`, `AngleDeltaUV ==
  AngleDeltaY == 0`) and route it through the same plane-dispatched
  `reconstruct_general_intra_directional_neighbour_block_into` (luma IDIF /
  chroma bilinear) the D135/D157 follow chroma already uses.
- Admit ONLY the verified subset: a row>0, NON-first-column full 64x64 superblock
  (`frontier.r != 0 && frontier.c != 0`, `n4w == 16`, `haveLeft && haveAbove`)
  D113 luma block and its `uv_mode == 0` directional-follow D113 chroma. Keep
  every other position (top-left, first-row `haveAbove == 0`, first-column,
  sub-partitioned, non-64x64) rejected, and keep the one-sided angles
  D45/D67/D203, non-zero angle deltas, and the directional-neighbour (`ctx != 0`)
  escape reorder rejected.
- Add the `syn-d113-intra-128x128-q80.ivf` fixture, its conformance manifest
  entry, the decoder support row, the decode matrix row, and the reciprocal
  LOCAL-REFERENCE-EVIDENCE entry.

## Impact

- Affected specs: `decode-general-intra-angle-d113`, `decoder-support`.
- Affected code: `crates/splot-decode/src/tile_payload/cdf/block_context.rs`,
  `crates/splot-decode/src/runtime_minimal/general_intra.rs`,
  `crates/splot-decode/src/runtime_minimal_recon.rs`.
- No dependency-graph change, no new dependency, no public CLI surface change.
