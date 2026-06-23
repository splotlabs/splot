# decode-general-intra-angle-d113 Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `decode-general-intra-angle-d113`.

## Requirements
### Requirement: General intra D113 vertical-leaning luma IDIF + follow chroma over a real above edge
The decoder SHALL reconstruct a general intra block whose luma is the § 7.13.2.8
`D113_PRED` (pAngle 113, canonical § 9.2 mode 5) MIDDLE directional mode with
`AngleDeltaY == 0` at a row>0, NON-first-column full 64x64 superblock block
(`frontier.r != 0 && frontier.c != 0`, `n4w == 16`, `haveLeft && haveAbove`),
building the § 7.13.2.1 edges from the partially-built frame: `LeftCol[i] =
CurrFrame[plane][y+i][x-1]` (the real reconstructed left column), `AboveRow[i] =
CurrFrame[plane][y-1][x+i]` (the real reconstructed above row), and the corner
`AboveRow[-1] == LeftCol[-1] == CurrFrame[plane][y-1][x-1]` (the real reconstructed
diagonally-above-left sample, `aboveMrlIndex == 0` at the superblock boundary,
`MrlIndex == 0`). D113 SHALL use `dx = Dr_Intra_Derivative[180 - 113] =
Dr_Intra_Derivative[67] = 24` and `dy = Dr_Intra_Derivative[113 - 90] =
Dr_Intra_Derivative[23] = 170` (§ 9.2). Because D113 is vertical-leaning, MOST
projections SHALL take the § 7.13.2.8 ABOVE branch (`base >= -(1 + mrlIndex)`) and
land on a NONZERO `shift`, so the luma § 7.13.2.8 IDIF 4-tap `Dr_Interp_Filter`
SHALL genuinely interpolate over the real above row + corner (unlike D135, whose
`shift == 0` reduces the IDIF to a sample copy). The decoder SHALL reconstruct the
matching `uv_mode == 0` directional-follow D113 chroma (`UVMode == D113_PRED`,
`AngleDeltaUV == AngleDeltaY == 0`) over its own row>0 `haveLeft && haveAbove`
chroma edges + corner via the `enableIdif == 0` bilinear branch. The decoder SHALL
add the § 5.20.7.27 residual (or write the bare prediction for an `all_zero` block)
for the luma and both chroma planes, guard the reconstruction by the § 8.2.4
`exit_symbol()` bit-exactness check, and SHALL NOT invoke AVM or dav2d. The decoder
SHALL reject — with a structured `decode/unsupported-feature` diagnostic — the
top-left, first-row (`haveAbove == 0`), first-column, sub-partitioned, and
non-64x64 D113 positions, the one-sided angles D45/D67/D203, a non-zero
`AngleDeltaY`, and the directional-neighbour (`ctx != 0`) escape reorder.

#### Scenario: A row>0 directional D113 luma + follow chroma block decodes to the oracle
- **WHEN** `splot decode` is given the committed full-grid intra key frame
  `syn-d113-intra-128x128-q80.ivf`, whose top-left / top-right / bottom-left 64x64
  superblocks are DC_PRED and whose BOTTOM-RIGHT 64x64 superblock
  (`frontier.r == 16`, `frontier.c == 16`, `haveLeft && haveAbove`) codes D113_PRED
  luma (via the § 5.20.5.3 `y_mode_offset` escape `y_mode_offset == 2` -> modeIdx 9
  -> `Reordered_Y_Mode[8] == D113_PRED`, § 8.3.2 ctx == 0) plus `uv_mode == 0`
  directional-follow D113 chroma, reading the real reconstructed above row, left
  column, and the diagonally-above-left corner `CurrFrame[plane][y-1][x-1]`
- **THEN** the decoder reconstructs the frame and its decoded-frame hash equals the
  pinned value `d32bc2b11585e7ea55f0d2401f18402c55e781c0a861bb613b55f5dc26a2a395`
- **AND** that output is byte-for-byte identical to the avmdec (`--rawvideo
  --i420`) and dav2d (`--demuxer ivf --muxer yuv`) raw decoder outputs (raw md5
  `ba857e73ad624d0409d1189b387d1ef7`)

#### Scenario: The D113 above branch exercises a nonzero-shift IDIF
- **WHEN** the bottom-right D113 block predicts its above-branch samples
  (`base >= -(1 + mrlIndex)`)
- **THEN** the § 7.13.2.8 luma IDIF 4-tap `Dr_Interp_Filter` interpolates over the
  real reconstructed above row + corner with a NONZERO `shift` for the majority of
  the block (2940 of the 4096 luma samples), so the block is genuinely directional
  (the bottom row varies across columns, not flat / row-constant)
- **AND** the left-branch (lower-left region, `dy == 170`) propagates the real
  reconstructed bottom-left right column up-right, so the block is non-flat (not DC)

#### Scenario: A still-unsupported angle stays rejected
- **WHEN** a general intra block is a one-sided directional mode (D45 value 3,
  D67 value 8, or D203 value 7), a D113 block at a top-left / first-row /
  first-column / sub-partitioned / non-64x64 position, carries a non-zero
  `AngleDeltaY`, or hits the directional-neighbour (`ctx != 0`) escape reorder
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic (only the verified row>0 non-first-column D113, plus the existing
  D135/D157/cardinal/SMOOTH subset, are in scope)

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed general intra fixtures, including
  the row>0 D135 `syn-d135row-intra-128x128-q80.ivf`, the first-row D157
  `syn-d157-intra-128x64-q80.ivf`, the cardinal `syn-vpred-intra-64x128-q160.ivf`,
  and the SMOOTH fixtures
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by adding the D113 path (the D135/D157/cardinal/SMOOTH arms are byte-identical)
