## ADDED Requirements

### Requirement: General intra D203 zone-3 one-sided luma + follow chroma reading the real left column
The decoder SHALL reconstruct a general intra block whose luma is the § 7.13.2.8
`D203_PRED` (pAngle 203, canonical § 9.2 mode 7) ZONE-3 one-sided directional mode
(`pAngle > 180`, `needBottom`) with `AngleDeltaY == 0` at a first-superblock-row,
NON-first-column full 64x64 superblock block (`frontier.r == 0 && frontier.c != 0`,
`n4w == 16`, `haveAbove == 0 && haveLeft == 1`), building the § 7.13.2.1 left edge
from the partially-built frame: `LeftCol[i] = CurrFrame[plane][Min(leftLimit, y+i)]
[x-1]` for `i` in `0..=maxBaseY` (`maxBaseY = w + h - 1`), with `leftLimit =
Min(maxY, y + h + 4 * num4BelowLeft - 1)` (`MrlIndex == 0`) and `num4BelowLeft` from
§ 5.20.7.25 `count_bottom_left_avail` over the § 5.20.2.3 `BlockDecoded` state
(`0` in raster order, so the below-left clamps), plus the corner `LeftCol[-1] =
CurrFrame[plane][y][x-1]` (the `haveAbove == 0 && haveLeft == 1` § 7.13.2.1 branch)
and the § 7.13.2.8 edge extensions (`LeftCol[-2] = LeftCol[-1]`,
`LeftCol[maxBaseY+1] = LeftCol[maxBaseY+2] = LeftCol[maxBaseY]`). D203 SHALL use
`dy = Dr_Intra_Derivative[270 - 203] = Dr_Intra_Derivative[67] = 24` (§ 9.2),
`idx = (j + 1) * dy`, `base = (idx >> 6) + i`, projecting DOWN-AND-LEFT into the
below-left so that the projection reads the real reconstructed left column (the
already-decoded left superblock's right column). The luma SHALL use the § 7.13.2.8
`enableIdif == 1` IDIF 4-tap `Dr_Interp_Filter` (`s = Σ(t=0..3)
Dr_Interp_Filter[shift][t] * LeftCol[base + t - 1]; pred = Clip1(Round2(s, 7))` for
`base <= maxBaseY`, else `pred = LeftCol[maxBaseY]`); because D203's `dy = 24` lands
most projections on a nonzero `shift`, the 4-tap genuinely interpolates over the
real reconstructed left column. The decoder SHALL reconstruct the matching
`uv_mode == 0` directional-follow D203 chroma (`UVMode == D203_PRED`,
`AngleDeltaUV == AngleDeltaY == 0`) over its own half-resolution left column via the
`enableIdif == 0` bilinear one-sided branch. The decoder SHALL keep `useIBP == 0`
(the fixture sets `enable_ibp == 0`, and § 7.13.2.7 gates `useIBP` on `pAngle >
180`) and `enable_intra_edge_filter == 0` / `MrlIndex == 0` (so the § 7.13.2.x
edge-filter / upsample synthesis is a no-op). The decoder SHALL add the § 5.20.7.27
residual (or write the bare prediction for an `all_zero` block) for the luma and
both chroma planes, guard the reconstruction by the § 8.2.4 `exit_symbol()`
bit-exactness check, and SHALL NOT invoke AVM or dav2d. The decoder SHALL reject —
with a structured `decode/unsupported-feature` diagnostic — the top-left,
first-column (no real left column), row>0, sub-partitioned, and non-64x64 D203
positions, the remaining one-sided angle D67, a non-zero `AngleDeltaY`, and the
directional-neighbour (`ctx != 0`) escape reorder.

#### Scenario: A first-row non-first-column zone-3 D203 luma + follow chroma block decodes to the oracle
- **WHEN** `splot decode` is given the committed intra key frame
  `syn-d203-intra-128x64-q80.ivf`, whose LEFT 64x64 superblock is DC_PRED carrying a
  vertical-gradient residual (so its right column reconstructs NON-FLAT) and whose
  RIGHT 64x64 superblock (`frontier.r == 0`, `frontier.c == 16`, `haveAbove == 0 &&
  haveLeft == 1`) codes D203_PRED luma (canonical § 9.2 mode 7, § 8.3.2 ctx == 0)
  plus `uv_mode == 0` directional-follow D203 chroma, reading the real reconstructed
  left column (`CurrFrame[plane][Min(leftLimit, y+i)][x-1]`)
- **THEN** the decoder reconstructs the frame and its decoded-frame hash equals the
  pinned value `3b95907f8808cc9d0bdd2eb376c8726019f7a4490cf8ecfcccab883fb11f8a3f`
- **AND** that output is byte-for-byte identical to the avmdec (`--rawvideo
  --i420`) and dav2d (`--demuxer ivf --muxer yuv`) raw decoder outputs (raw md5
  `2789636ec6bca9efcac829bbd7ca3712`)

#### Scenario: The D203 projection reads the real reconstructed left column
- **WHEN** the right D203 block predicts a sample whose `base = ((j + 1) * 24 >> 6) +
  i` projects down-and-left into the lower part of the left column (e.g.
  `pred[0][63]`, `base = 24`)
- **THEN** the prediction reads the REAL reconstructed left column (the left
  superblock's non-flat right column gradient) at a row well below the top, so the
  top-right sample is a much higher gradient value than the top-left
  (`pred[0][63] > pred[0][0] + 30`), NOT a flat fallback a middle/cardinal angle
  would produce
- **AND** the block reconstructs as a genuinely directional surface (181 distinct
  values)

#### Scenario: A still-unsupported one-sided angle stays rejected
- **WHEN** a general intra block is the not-yet-supported one-sided directional mode
  D67 (value 8), a D203 block at a top-left / first-column / row>0 /
  sub-partitioned / non-64x64 position, carries a non-zero `AngleDeltaY`, or hits
  the directional-neighbour (`ctx != 0`) escape reorder
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic (only the verified first-row non-first-column D203, plus the existing
  D45/D113/D135/D157/cardinal/SMOOTH subset, are in scope)

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed general intra fixtures, including
  the zone-1 D45 `syn-d45-intra-192x128-q80.ivf`, the D113
  `syn-d113-intra-128x128-q80.ivf`, the cardinal `syn-vpred-intra-64x128-q160.ivf`,
  and the SMOOTH fixtures
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by adding the D203 path (the D45/D113/D135/D157/cardinal/SMOOTH/DC arms are
  byte-identical)
