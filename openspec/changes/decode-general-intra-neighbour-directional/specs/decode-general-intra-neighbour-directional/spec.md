## ADDED Requirements

### Requirement: General intra neighbour-having directional (D135) luma and follow chroma over a real edge
The decoder SHALL reconstruct a first-superblock-row (`frontier.r == 0`,
`haveAbove == 0`), non-top-left, full 64x64 superblock (`n4w == 16`) general intra
block whose luma is § 7.13.2.8 `D135_PRED` (decoded via the § 5.20.5.3
`y_mode_offset` escape over a NON-directional joint-mode neighbour, so the § 8.3.2
context is `0`) and whose chroma is the `uv_mode == 0` directional-follow D135
mode (`UVMode == D135_PRED`, `AngleDeltaUV == AngleDeltaY == 0`), reading the REAL
reconstructed left column of the already-decoded left neighbour. For pAngle 135
`dx = dy = Dr_Intra_Derivative[45] = 64`, so every § 7.13.2.8 projection has
`shift == 0`: the luma IDIF 4-tap (`enableIdif == 1`, `Dr_Interp_Filter[0] =
{0, 128, 0, 0}`) reduces to the sample copy `Edge[base]`, bit-identical to the
chroma bilinear branch (`enableIdif == 0`) even over a non-flat reconstructed
edge. The decoder SHALL build the logical `AboveRow[-1..w)` / `LeftCol[-1..h)`
edges from the partially-built frame faithful to § 7.13.2.1 for the minimal-tool
subset (`MrlIndex == 0`, `enable_intra_edge_filter == 0`, no DIP/upsample): for
`haveLeft == 1, haveAbove == 0`, `LeftCol[i]` is the real reconstructed left column
(bottom-left sentinel clamped, `num4BelowLeft == 0`), `AboveRow[i]` is the repeated
first left sample `CurrFrame[plane][y][x - 1]`, and the corner
`AboveRow[-1] = LeftCol[-1] = CurrFrame[plane][y][x - 1]`. The decoder SHALL run
the § 7.13.2.8 middle-angle prediction over these edges (the shared bilinear
predictor, bit-exact for D135) and add the § 5.20.7.27 residual (or write the bare
prediction for an `all_zero` block) for the luma and both chroma planes. The
reconstruction SHALL be guarded by the § 8.2.4 `exit_symbol()` bit-exactness check
and SHALL NOT invoke AVM or dav2d. The decoder SHALL reject — with a structured
`decode/unsupported-feature` diagnostic — a row>0 D135 block (which reads the real
reconstructed above row), a sub-superblock (split) directional block, a directional
NEIGHBOUR (`ctx != 0`) D135 escape (the § 5.20.5.3 directional-neighbour reorder),
and other directional angles or non-zero angle deltas (where `shift != 0` so the
real § 7.13.2.8 IDIF 4-tap genuinely differs from bilinear).

#### Scenario: A neighbour-having D135 luma and follow chroma block decodes to the oracle
- **WHEN** `splot decode` is given the committed multi-superblock intra key frame
  `syn-rdir-intra-128x64-q80.ivf`, whose LEFT 64x64 superblock codes as
  `SMOOTH_V_PRED` luma with DC chroma and whose RIGHT 64x64 superblock codes as
  `D135_PRED` luma plus `uv_mode == 0` directional-follow D135 chroma, reading the
  LEFT superblock's real reconstructed right column as its § 7.13.2.1 left edge
- **THEN** the general intra path reconstructs the right D135 luma block and both
  the U and V directional-follow D135 chroma blocks over the real reconstructed
  left column plus residual, and succeeds
- **AND** the decoded output matches the avmdec (`--rawvideo --i420`) and dav2d
  (`--demuxer ivf`) raw outputs byte-for-byte (md5
  `9ff7e4d46c0dd4fa979070ce4ca4dd1c`)
- **AND** the decoded-frame hash is the pinned
  `9ea9254abc7d7507558099d5ae3e78eaf5d88625e1cc8184038321650b2b54a4`

#### Scenario: The right block is a genuine directional reconstruction over a non-flat edge
- **WHEN** the right D135 luma block is reconstructed reading the LEFT superblock's
  real reconstructed right column (a non-flat vertical gradient with 34 distinct
  values)
- **THEN** the right superblock's top row varies across columns (a genuine
  135-degree directional pattern, not flat/row-constant), proving the § 7.13.2.8
  directional predictor ran over the real reconstructed edge rather than over the
  flat fallback

#### Scenario: A non-D135 directional right block stays rejected
- **WHEN** a neighbour-having general intra block's luma resolves to a directional
  mode other than `D135_PRED` (or its directional-follow chroma is not the D135
  follow)
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic (only `D135_PRED`, whose `shift == 0` makes IDIF equal bilinear, is in
  scope; other angles need the real § 7.13.2.8 IDIF 4-tap)

#### Scenario: A row>0 / sub-superblock / directional-neighbour D135 block stays rejected
- **WHEN** a D135 block reads the real reconstructed above row (superblock row > 0),
  or is a sub-superblock (split) directional block, or is the `y_mode_offset` escape
  over a directional joint-mode neighbour (`ctx != 0`)
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic, deferring those paths until an oracle fixture pins them

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed general intra fixtures, including
  the no-neighbour D135 `syn-hedge-intra-64x64-q80.ivf` and the directional-follow
  chroma `syn-dfchroma-intra-64x64-q80.ivf`
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by adding the neighbour-having directional path
