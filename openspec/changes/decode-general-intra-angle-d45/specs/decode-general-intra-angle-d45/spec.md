## ADDED Requirements

### Requirement: General intra D45 zone-1 one-sided luma + follow chroma reading the real above-right
The decoder SHALL reconstruct a general intra block whose luma is the § 7.13.2.8
`D45_PRED` (pAngle 45, canonical § 9.2 mode 3) ZONE-1 one-sided directional mode
(`pAngle < 90`, `needRight`) with `AngleDeltaY == 0` at a row>0, NON-first-column,
NON-rightmost full 64x64 superblock block (`frontier.r != 0 && frontier.c != 0`,
`n4w == 16`, `full_sb_num4_above_right > 0`, `haveLeft && haveAbove`), building the
§ 7.13.2.1 above edge from the partially-built frame: `AboveRow[i] =
CurrFrame[plane][y-1][Min(aboveLimit, x+i)]` for `i` in `0..=maxBaseX`
(`maxBaseX = w + h - 1`), with `aboveLimit = Min(maxX, x + w + 4 * num4AboveRight -
1)` (`MrlIndex == 0`, `aboveMrlIndex == 0`) and `num4AboveRight` from § 5.20.7.25
`count_top_right_avail` over the § 5.20.2.3 `BlockDecoded` state, plus the corner
`AboveRow[-1] = CurrFrame[plane][y-1][x-1]` and the § 7.13.2.8 edge extensions
(`AboveRow[-2] = AboveRow[-1]`, `AboveRow[maxBaseX+1] = AboveRow[maxBaseX+2] =
AboveRow[maxBaseX]`). D45 SHALL use `dx = Dr_Intra_Derivative[45] = 64` (§ 9.2),
`idx = (i + 1) * dx`, `base = (idx >> 6) + j = (i + 1) + j`, projecting
UP-AND-RIGHT into the above-right so that the upper-right triangle reads the real
reconstructed above-right samples (the bottom row of the diagonally-above-right
superblock). The luma SHALL use the § 7.13.2.8 `enableIdif == 1` IDIF 4-tap
`Dr_Interp_Filter` (`s = Σ(t=0..3) Dr_Interp_Filter[shift][t] * AboveRow[base + t -
1]; pred = Clip1(Round2(s, 7))` for `base <= maxBaseX`, else `pred =
AboveRow[maxBaseX]`); because every D45 projection has `shift == 0`, the 4-tap
reduces to the sample copy `AboveRow[base]`, bit-identical to the bilinear branch
but still reading far into the real above-right. The decoder SHALL reconstruct the
matching `uv_mode == 0` directional-follow D45 chroma (`UVMode == D45_PRED`,
`AngleDeltaUV == AngleDeltaY == 0`) over its own half-resolution row>0
non-rightmost above + above-right via the `enableIdif == 0` bilinear one-sided
branch. The decoder SHALL keep `useIBP == 0` (the fixture sets `enable_ibp == 0`,
and § 7.13.2.7 gates `useIBP` on `pAngle < 90`) and `enable_intra_edge_filter ==
0` / `MrlIndex == 0` (so the § 7.13.2.x edge-filter / upsample synthesis is a
no-op). The decoder SHALL add the § 5.20.7.27 residual (or write the bare
prediction for an `all_zero` block) for the luma and both chroma planes, guard the
reconstruction by the § 8.2.4 `exit_symbol()` bit-exactness check, and SHALL NOT
invoke AVM or dav2d. The decoder SHALL reject — with a structured
`decode/unsupported-feature` diagnostic — the top-left, first-row (`haveAbove ==
0`), first-column, RIGHTMOST (no decoded above-right), sub-partitioned, and
non-64x64 D45 positions, the other one-sided angles D67/D203, a non-zero
`AngleDeltaY`, and the directional-neighbour (`ctx != 0`) escape reorder.

#### Scenario: A row>0 non-rightmost zone-1 D45 luma + follow chroma block decodes to the oracle
- **WHEN** `splot decode` is given the committed full-grid intra key frame
  `syn-d45-intra-192x128-q80.ivf`, whose top-left / top-middle / bottom-left /
  bottom-right 64x64 superblocks are DC_PRED, whose top-right DC superblock carries
  a horizontal-gradient residual (so it reconstructs NON-FLAT), and whose
  BOTTOM-MIDDLE 64x64 superblock (`frontier.r == 16`, `frontier.c == 16`,
  `haveLeft && haveAbove`, NON-rightmost) codes D45_PRED luma (via the § 5.20.5.3
  `y_mode_offset` escape `y_mode_offset == 0` -> modeIdx 7 -> `Reordered_Y_Mode[5]
  == D45_PRED`, § 8.3.2 ctx == 0) plus `uv_mode == 0` directional-follow D45 chroma,
  reading the real reconstructed above row + above-right
  (`CurrFrame[plane][y-1][Min(aboveLimit, x+i)]`) + the diagonally-above-left corner
- **THEN** the decoder reconstructs the frame and its decoded-frame hash equals the
  pinned value `d08056c0d1ed3f379e3072c7f1ebced04da0f6df994efd0b5f8d39b76c0b683f`
- **AND** that output is byte-for-byte identical to the avmdec (`--rawvideo
  --i420`) and dav2d (`--demuxer ivf --muxer yuv`) raw decoder outputs (raw md5
  `8fe6a134c01b0d20be4016348ccd3b99`)

#### Scenario: The D45 upper-right triangle reads the real reconstructed above-right
- **WHEN** the bottom-middle D45 block predicts its upper-right-triangle samples
  (`base = (i + 1) + j >= w`, e.g. `pred[0][63] = AboveRow[64]`)
- **THEN** the prediction reads the REAL reconstructed above-right (the top-right
  superblock's non-flat bottom row, 32 distinct values 42..228), NOT the flat
  above-middle row (128) the middle angles would clamp to
- **AND** the block reconstructs as a genuinely directional surface (33 distinct
  values, the lower-left triangle copying the flat above-middle and the upper-right
  triangle the above-right gradient)

#### Scenario: A still-unsupported one-sided angle stays rejected
- **WHEN** a general intra block is a not-yet-supported one-sided directional mode
  (D67 value 8 or D203 value 7), a D45 block at a top-left / first-row /
  first-column / RIGHTMOST / sub-partitioned / non-64x64 position, carries a
  non-zero `AngleDeltaY`, or hits the directional-neighbour (`ctx != 0`) escape
  reorder
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic (only the verified row>0 non-first-column non-rightmost D45, plus the
  existing D113/D135/D157/cardinal/SMOOTH subset, are in scope)

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed general intra fixtures, including
  the D113 `syn-d113-intra-128x128-q80.ivf`, the row>0 D135
  `syn-d135row-intra-128x128-q80.ivf`, the first-row D157
  `syn-d157-intra-128x64-q80.ivf`, the cardinal `syn-vpred-intra-64x128-q160.ivf`,
  and the SMOOTH fixtures
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by adding the D45 path (the D113/D135/D157/cardinal/SMOOTH/DC arms are
  byte-identical)
