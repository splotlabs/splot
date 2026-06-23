## ADDED Requirements

### Requirement: General intra row>0 directional reconstruction reads the real § 7.13.2.1 corner
The decoder SHALL reconstruct a general intra block whose luma is the § 7.13.2.8
`D135_PRED` (pAngle 135) middle directional mode with `AngleDeltaY == 0` at a
row>0, NON-first-column full 64x64 superblock block (`frontier.r != 0 &&
frontier.c != 0`, `n4w == 16`, `haveLeft && haveAbove`), building the § 7.13.2.1
edges from the partially-built frame: `LeftCol[i] = CurrFrame[plane][y+i][x-1]`
(the real reconstructed left column, clamped at the bottom-left sentinel,
`num4BelowLeft == 0` in raster order), `AboveRow[i] = CurrFrame[plane][y-1][x+i]`
(the real reconstructed above row), and the corner
`AboveRow[-1] == LeftCol[-1] == CurrFrame[plane][y-1][x-1]` (the real reconstructed
diagonally-above-left sample, read via the current-frame workspace accessor, with
`aboveMrlIndex == 0` at the superblock boundary and `MrlIndex == 0`). § 7.13.2.8
D135 SHALL read that corner on its main diagonal (`column == row`, `above_base ==
-1`, `shift == 0`, a sample copy). The decoder SHALL reconstruct the matching
`uv_mode == 0` directional-follow D135 chroma (`UVMode == D135_PRED`, `AngleDeltaUV
== AngleDeltaY == 0`) over its own row>0 `haveLeft && haveAbove` chroma edges +
corner. The decoder SHALL add the § 5.20.7.27 residual (or write the bare
prediction for an `all_zero` block) for the luma and both chroma planes, guard the
reconstruction by the § 8.2.4 `exit_symbol()` bit-exactness check, and SHALL NOT
invoke AVM or dav2d. Because pAngle 135 has every projection `shift == 0`, the
luma § 7.13.2.8 IDIF 4-tap (`Dr_Interp_Filter[0] == {0, 128, 0, 0}`) SHALL reduce
to the sample copy `Edge[base]`, bit-identical to the chroma `enableIdif == 0`
bilinear branch even over the non-flat real edge. The decoder SHALL reject — with a
structured `decode/unsupported-feature` diagnostic — a row>0 FIRST-column
(`!haveLeft && haveAbove`) directional block, any row>0 D157 block, a
sub-superblock (split-child) directional block, a non-zero `AngleDeltaY`, and the
directional-neighbour (`ctx != 0`) escape reorder.

#### Scenario: A row>0 directional D135 luma + follow chroma block decodes to the oracle
- **WHEN** `splot decode` is given the committed full-grid intra key frame
  `syn-d135row-intra-128x128-q80.ivf`, whose top-left / top-right / bottom-left
  64x64 superblocks are DC_PRED and whose BOTTOM-RIGHT 64x64 superblock
  (`frontier.r == 16`, `frontier.c == 16`, `haveLeft && haveAbove`) codes D135_PRED
  luma (via the § 5.20.5.3 `y_mode_offset` escape, § 8.3.2 ctx == 0) plus
  `uv_mode == 0` directional-follow D135 chroma, reading the real reconstructed
  above row, left column, and the diagonally-above-left corner
  `CurrFrame[plane][y-1][x-1]`
- **THEN** the decoder reconstructs the frame and its decoded-frame hash equals the
  pinned value `85583e5a46ac6a2db97854b86f643735c1b9710bee2c2d2bc65d1aa5a16fe3a1`
- **AND** that output is byte-for-byte identical to the avmdec (`--rawvideo
  --i420`) and dav2d (`--demuxer ivf --muxer yuv`) raw decoder outputs (raw md5
  `79bd663383515e37b75b1ad7054c84d6`)

#### Scenario: The row>0 directional main diagonal copies the real corner
- **WHEN** the bottom-right D135 block predicts its main diagonal samples
  (`column == row`, `above_base == -1`)
- **THEN** each main-diagonal sample equals the real reconstructed corner
  `CurrFrame[Y][63][63] == 100` (the top-left superblock's bottom-right sample), not
  the no-neighbour fallback corner `128`
- **AND** the left-branch (`j < i`) propagates the real reconstructed bottom-left
  right column up-right, so the block is non-flat (not DC)

#### Scenario: A still-unsupported row>0 directional position stays rejected
- **WHEN** a general intra block is a row>0 FIRST-column (`!haveLeft &&
  haveAbove`) D135 block, a row>0 D157 block, a sub-superblock directional block,
  carries a non-zero `AngleDeltaY`, or hits the directional-neighbour (`ctx != 0`)
  escape reorder
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic (only the verified row>0 non-first-column D135, plus the first-row /
  no-neighbour directional subset, are in scope)

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed general intra fixtures, including
  the no-neighbour D135 `syn-hedge-intra-64x64-q80.ivf`, the first-row
  neighbour-having D135 `syn-rdir-intra-128x64-q80.ivf`, and the first-row D157
  `syn-d157-intra-128x64-q80.ivf`
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by adding the real-corner row>0 directional path (the no-corner first-row /
  no-neighbour arms are byte-identical)
