## ADDED Requirements

### Requirement: General intra D157 luma IDIF and follow chroma over a real edge
The decoder SHALL reconstruct a general intra block whose luma is the § 7.13.2.8
`D157_PRED` (pAngle 157) middle directional mode with `AngleDeltaY == 0`, applying
the § 7.13.2.8 / § 9.2 `Dr_Interp_Filter[32][4]` luma IDIF 4-tap interpolation
(`enableIdif == 1`): for each predicted sample
`s = Σ(t=0..3) Dr_Interp_Filter[shift][t] * Edge[base + t - 1]` and
`pred[i][j] = Clip1(Round2(s, 7))`, where `Edge` is `LeftCol` or `AboveRow` with
`base` and `shift = (idx >> 1) & 0x1F` derived per § 7.13.2.8 (`dx =
Dr_Intra_Derivative[23] = 170`, `dy = Dr_Intra_Derivative[67] = 24`). The 4-tap
sum is signed; `Round2` SHALL floor it and `Clip1` SHALL clamp a negative result to
0. The decoder SHALL reconstruct the matching `uv_mode == 0` directional-follow
D157 chroma (`UVMode == D157_PRED`, `AngleDeltaUV == AngleDeltaY == 0`) through the
§ 7.13.2.8 `enableIdif == 0` bilinear branch (chroma is never IDIF). The block SHALL
be decoded via the § 5.20.5.3 `y_mode_offset` escape over a NON-directional
joint-mode neighbour (the § 8.3.2 context is `0`), at a first-superblock-row,
non-first-column full 64x64 superblock block (`frontier.r == 0`,
`frontier.c != 0`, `haveLeft && !haveAbove`), reading the real reconstructed
§ 7.13.2.1 left column of the already-decoded left neighbour, with the corner
`AboveRow[-1] == LeftCol[-1]` the repeated first left sample. The decoder SHALL add
the § 5.20.7.27 residual (or write the bare prediction for an `all_zero` block) for
the luma and both chroma planes, guard the reconstruction by the § 8.2.4
`exit_symbol()` bit-exactness check, and SHALL NOT invoke AVM or dav2d. At
`shift == 0` the filter row `Dr_Interp_Filter[0] == {0, 128, 0, 0}` SHALL reduce the
4-tap to a sample copy, so the luma `D135_PRED` path (every projection
`shift == 0`) SHALL remain byte-identical. The decoder SHALL reject — with a
structured `decode/unsupported-feature` diagnostic — a top-left no-neighbour,
first-column, sub-superblock, or row>0 D157 position; the other middle angle
`D113_PRED` and the one-sided angles `D45_PRED` / `D67_PRED` / `D203_PRED`; a
non-zero `AngleDeltaY`; the directional-neighbour (`ctx != 0`) escape reorder; and
a non-64x64 superblock block.

#### Scenario: A neighbour-having D157 luma IDIF and follow chroma block decodes to the oracle
- **WHEN** `splot decode` is given the committed multi-superblock intra key frame
  `syn-d157-intra-128x64-q80.ivf`, whose LEFT 64x64 superblock codes SMOOTH_V_PRED
  luma with DC chroma and whose RIGHT 64x64 superblock (frontier col 16) codes
  D157_PRED luma (via the `y_mode_offset` escape, ctx 0) plus `uv_mode == 0`
  directional-follow D157 chroma, reading the LEFT superblock's real reconstructed
  right column as its § 7.13.2.1 left edge
- **THEN** the decoder reconstructs the frame and its decoded-frame hash equals the
  pinned value `bf93ca6b8f55e1fb7db2584f3e3821ad67f21018b774c6e326634362ee5ef046`
- **AND** that output is byte-for-byte identical to the avmdec (`--rawvideo
  --i420`) and dav2d (`--demuxer ivf --muxer yuv`) raw decoder outputs (raw md5
  `c8698fdb7628843971bc9e37a82391ae`)

#### Scenario: The luma IDIF 4-tap genuinely interpolates at a nonzero shift
- **WHEN** the D157 luma block projects samples through the § 7.13.2.8 left branch
  with `shift != 0` (2940 of its 3344 left-branch samples)
- **THEN** the reconstruction applies the `Dr_Interp_Filter` 4-tap (not the 2-tap
  bilinear), and the result differs from the bilinear branch over the same edge
- **AND** the `splot-recon` `predict_intra_middle_directional_angle_rect_idif_into`
  primitive computes the same value for known inputs (unit/property-tested),
  reduces to a sample copy at `shift == 0` (D135), and clamps a negative 4-tap sum
  to 0

#### Scenario: A still-unsupported angle or D157 position stays rejected
- **WHEN** a general intra block's luma resolves to the middle angle `D113_PRED` or
  a one-sided angle (`D45_PRED` / `D67_PRED` / `D203_PRED`), or carries a non-zero
  `AngleDeltaY`, or is a D157 block at the top-left / first-column / sub-superblock
  / row>0 position
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic (only the verified D135 and the first-superblock-row, non-first-column
  D157 are in scope)

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed general intra fixtures, including
  the no-neighbour D135 `syn-hedge-intra-64x64-q80.ivf`, the neighbour-having D135
  `syn-rdir-intra-128x64-q80.ivf`, and the cardinal `syn-vpred-intra-64x128-q160.ivf`
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by adding the luma IDIF 4-tap (the D135 `shift == 0` reduction keeps the luma
  D135 path byte-identical)
