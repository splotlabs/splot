# decode-general-intra-cardinal Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `decode-general-intra-cardinal`.

## Requirements
### Requirement: General intra cardinal V_PRED / H_PRED luma and follow chroma over a real edge
The decoder SHALL reconstruct a general intra block whose luma is the cardinal
§ 7.13.2.8 `V_PRED` (pAngle 90, step 4) or `H_PRED` (pAngle 180, step 5)
directional mode with `AngleDeltaY == 0`, decoded via the § 5.20.5.3 DIRECT
first-mode-set `y_mode_index` (`y_mode_set == 0`,
`NON_DIRECTIONAL_MODES_COUNT <= y_mode_index < MODE_INDEX_COUNT - 1`, so no
`y_mode_offset` escape) over a NON-directional joint-mode neighbour (the § 8.3.2
context is `0`): `V_PRED` at `y_mode_index == 5` (`get_intra_y_mode_set(5)` →
`Default_Mode_List_Y[0] == 17` → `modeDelta 22` → `Reordered_Y_Mode[7] == V_PRED`)
and `H_PRED` at `y_mode_index == 6` (`Default_Mode_List_Y[1] == 45` →
`modeDelta 50` → `Reordered_Y_Mode[11] == H_PRED`). The decoder SHALL reconstruct
the matching `uv_mode == 0` directional-follow chroma (`UVMode == V_PRED` /
`H_PRED`, `AngleDeltaUV == AngleDeltaY == 0`). `V_PRED` is `pred[i][j] =
AboveRow[j]` (every row copies the real reconstructed § 7.13.2.1 above row of the
already-decoded above neighbour, `haveAbove == 1`); `H_PRED` is `pred[i][j] =
LeftCol[i]` (every column copies the real reconstructed left column of the
already-decoded left neighbour, `haveLeft == 1`). The cardinal copy reads no
corner, no opposite edge, no IDIF, and runs no `useIBP` (§ 7.13.2.7 gates `useIBP`
on `pAngle < 90 || pAngle > 180`, and its edge-filter step is skipped for
`pAngle == 90 || pAngle == 180`), so it is bit-exact over a non-flat reconstructed
edge without interpolation. The decoder SHALL add the § 5.20.7.27 residual (or
write the bare prediction for an `all_zero` block) for the luma and both chroma
planes, guard the reconstruction by the § 8.2.4 `exit_symbol()` bit-exactness
check, and SHALL NOT invoke AVM or dav2d. The decoder SHALL reject — with a
structured `decode/unsupported-feature` diagnostic — a first-superblock-row V_PRED
or first-superblock-column H_PRED (which would read the § 7.13.2.1 no-neighbour
fallback), a sub-superblock (split) cardinal block, a directional-neighbour
(`ctx != 0`) escape/reorder, a non-cardinal directional mode, a non-zero
`AngleDeltaY`, the `y_second_mode` (`y_mode_set != 0`) path, and a non-64x64
superblock block.

#### Scenario: A neighbour-having V_PRED luma and follow chroma block decodes to the oracle
- **WHEN** `splot decode` is given the committed single-column multi-superblock-row
  intra key frame `syn-vpred-intra-64x128-q160.ivf`, whose TOP 64x64 superblock
  codes DC_PRED and whose BOTTOM 64x64 superblock (frontier row 16) codes V_PRED
  luma (via `y_mode_set == 0, y_mode_index == 5`) plus `uv_mode == 0`
  directional-follow V_PRED chroma, reading the TOP superblock's real reconstructed
  bottom row as its § 7.13.2.1 above edge
- **THEN** the general intra path reconstructs the bottom V_PRED luma block and both
  the U and V directional-follow chroma blocks over the real reconstructed above row
  plus residual, and succeeds
- **AND** the decoded output matches the avmdec (`--rawvideo --i420`) and dav2d
  (`--demuxer ivf --muxer yuv`) raw outputs byte-for-byte (md5
  `d35b827668076a934bb6c21717f9a8f9`)
- **AND** the decoded-frame hash is the pinned
  `5b2761c0d2eb2502af5cbe544b2cadbb676a4b84b60953d86a3e42d7df910e39`

#### Scenario: A neighbour-having H_PRED luma and follow chroma block decodes to the oracle
- **WHEN** `splot decode` is given the committed multi-superblock intra key frame
  `syn-hpred-intra-128x64-q180.ivf`, whose LEFT 64x64 superblock codes DC_PRED and
  whose RIGHT 64x64 superblock (frontier col 16) codes H_PRED luma (via
  `y_mode_set == 0, y_mode_index == 6`) plus `uv_mode == 0` directional-follow
  H_PRED chroma, reading the LEFT superblock's real reconstructed right column as
  its § 7.13.2.1 left edge
- **THEN** the general intra path reconstructs the right H_PRED luma block and both
  chroma blocks over the real reconstructed left column plus residual, and succeeds
- **AND** the decoded output matches the avmdec and dav2d raw outputs byte-for-byte
  (md5 `aac61b219518ce5057a6284262ac3bb9`)
- **AND** the decoded-frame hash is the pinned
  `826cea4e59f8280b538c3efc26e7be72cd1912aa19f235ebf3f862fc8832a885`

#### Scenario: The cardinal block is a genuine vertical / horizontal copy, not DC
- **WHEN** the V_PRED bottom superblock (or H_PRED right superblock) is
  reconstructed reading the real reconstructed above row (or left column)
- **THEN** for V_PRED each column is constant down the block while the columns vary
  across the width (a vertical copy of the column-varying above row, ruling out a
  flat DC reconstruction); for H_PRED each row is constant across the block while
  the rows vary down the height
- **AND** the boundary samples continue the neighbour pattern (near-continuous
  seam), proving the real reconstructed edge was read rather than the § 7.13.2.1
  no-neighbour flat fallback

#### Scenario: A non-cardinal directional first-set block stays rejected
- **WHEN** a general intra block's luma resolves (via the first-mode-set
  `y_mode_index` or the `y_mode_offset` escape) to a directional mode other than
  cardinal `V_PRED` / `H_PRED` (e.g. `D45_PRED`), or carries a non-zero
  `AngleDeltaY`
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic (only the cardinal D135 middle-angle and the cardinal V/H copies are
  in scope)

#### Scenario: A first-row / first-column / sub-superblock / directional-neighbour cardinal block stays rejected
- **WHEN** a V_PRED block is on the first superblock row (no real above row), or an
  H_PRED block is on the first superblock column (no real left column), or a
  cardinal block is a sub-superblock (split) block, or the mode is selected over a
  directional joint-mode neighbour (`ctx != 0`)
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic, deferring those paths until an oracle fixture pins them

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed general intra fixtures, including
  the no-neighbour D135 `syn-hedge-intra-64x64-q80.ivf`, the neighbour-having D135
  `syn-rdir-intra-128x64-q80.ivf`, and the SMOOTH_V grid
  `syn-vgrid-intra-192x128-q120.ivf`
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by adding the cardinal directional path
