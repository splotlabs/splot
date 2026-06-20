## ADDED Requirements

### Requirement: General intra multi-block non-DC luma decode over a reconstructed neighbour edge
The decoder SHALL reconstruct a non-DC § 7.13.2.13 `SMOOTH_V_PRED` /
`SMOOTH_H_PRED` luma prediction block that has an already-decoded reconstructed
neighbour, for a full 64x64 superblock block on the general intra path, reading
the REAL reconstructed neighbour edge (not the no-neighbour flat fallback). It
SHALL build the § 7.13.2.1 `LeftCol[0..=h]` and `AboveRow[0..=w]` edges from the
partially-built frame: the reconstructed left column when `haveLeft` (with the
bottom-left sentinel `LeftCol[h]` clamped to the last in-block left sample, since
`num4BelowLeft == 0` in raster order), the reconstructed above row when
`haveAbove` (with the § 7.13.2.1 top-right sentinel `AboveRow[w]` reading the real
reconstructed above-right sample when `num4AboveRight > 0` and in-frame), and the
§ 7.13.2.1 no-above (`AboveRow[i] = CurrFrame[plane][y][x-1]`), no-left
(`LeftCol[i] = CurrFrame[plane][y-1][x]`), and no-neighbour fallbacks otherwise.
It SHALL run the shared `splot-recon` § 7.13.2.13 smooth predictor for the decoded
SMOOTH_V/H mode over those edges, and SHALL add the § 5.20.7.27 residual over that
per-sample prediction. It SHALL validate § 8.2.4 `exit_symbol()` after the
coefficients. The no-neighbour top-left non-DC / directional luma path SHALL
remain unchanged. It SHALL gate the neighbour-edge non-DC luma block to a full
64x64 superblock (`n4w == 16`), rejecting a neighbour-having § 7.13.2.8
directional (`D135`) luma block (which needs the real § 7.13.2.8 IDIF 4-tap
interpolation over a non-flat edge), a sub-superblock non-DC block, and the
not-yet-verified luma / chroma modes with a structured
`decode/unsupported-feature` diagnostic before any reconstruction. It SHALL NOT
handle the in-frame directional-neighbour `y_mode_index` reorder, sub-superblock
non-DC blocks, non-64x64-superblock non-DC neighbour edges, inter prediction,
in-loop filters, or invoke AVM or dav2d.

#### Scenario: Multi-block SMOOTH_V neighbour-edge frame decodes to the oracle
- **WHEN** `splot decode` is given the committed multi-superblock intra key frame
  `syn-mbvg-intra-128x64-q80.ivf`
- **THEN** the general intra path reconstructs the left (top-left, no-neighbour)
  64x64 superblock as `SMOOTH_V_PRED` over the § 7.13.2.1 flat fallback edges and
  the right 64x64 superblock as `SMOOTH_V_PRED` over the REAL reconstructed
  left-neighbour edge plus the § 5.20.7.27 residual, with flat DC chroma, and
  succeeds
- **AND** the reconstructed luma is a genuinely non-flat reconstruction whose two
  superblocks differ (the right superblock reads the real neighbour edge, not a
  copy of the left) and matches the avmdec and dav2d raw outputs (md5
  `3e57ba0c8cbdbe1d3400b0ae365c5d8e`)
- **AND** the decoded-frame hash is the pinned
  `269b4969800751c63f7f0605f1f7b8f178f7bf85590ec62fe64313ff394d6dfd`

#### Scenario: Unsupported neighbour-having non-DC cases are rejected before reconstruction
- **WHEN** a neighbour-having block uses a § 7.13.2.8 directional luma mode, a
  non-DC luma mode on a block smaller than the 64x64 superblock, or another
  not-yet-verified luma or chroma mode
- **THEN** the decoder returns a structured `decode/unsupported-feature`
  diagnostic without reconstructing the block

#### Scenario: Multi-superblock-row neighbour SMOOTH is deferred
- **WHEN** a non-top-left SMOOTH luma block is at superblock row > 0 (so it has a
  real above neighbour and a real §7.13.2.1 above-right sentinel)
- **THEN** the decoder returns a structured `decode/unsupported-feature`
  diagnostic (the neighbour SMOOTH path is verified only for the first superblock
  row, where the above edge is the no-neighbour fallback)
