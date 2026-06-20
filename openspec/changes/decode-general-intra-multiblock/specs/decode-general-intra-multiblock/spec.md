## ADDED Requirements

### Requirement: General intra multi-block partition decode
The decoder SHALL decode a split-partition multi-block 64x64 8-bit 4:2:0 intra
key frame on the general intra path. It SHALL walk the complete AV2 § 5.20.3.1
partition tree depth-first, reading partition-split symbols and per-block syntax
interleaved on one live symbol decoder and the tile CDFs, and maintaining the
§ 5.20.4.1 MI-size partition context across blocks. At each `PARTITION_NONE`
leaf it SHALL decode the § 5.20.5.3 mode info and the § 5.20.7.27 Y/U/V
coefficients, deriving the § 8.3.2 `txb_skip` context from a persistent
coefficient neighbour context threaded across blocks, and SHALL reconstruct the
block into a persistent frame workspace in decode order so each non-first block's
§ 7.13.2 DC prediction reads its already-reconstructed above/left neighbours
(`128` fallback when none). It SHALL validate § 8.2.4 `exit_symbol()` after the
whole tile. It SHALL be gated to DC_PRED square blocks and SHALL reject non-DC
modes and non-square (rectangular leaf) partitions with a structured
`decode/unsupported-feature` diagnostic. The single-block general intra decode
SHALL be unified through this driver. It SHALL NOT handle multiple tiles,
non-64x64 frames, chroma `cctx`/CfL, inter prediction, in-loop filters, or invoke
AVM or dav2d.

#### Scenario: Four-quadrant frame decodes to the oracle
- **WHEN** `splot decode` is given the committed multi-block intra key frame
  `syn-quad-intra-64x64-q80.ivf`
- **THEN** the general intra path walks the partition tree into four square 32x32
  DC_PRED blocks, decoding and reconstructing each in decode order, and succeeds
- **AND** the reconstructed luma quadrants are 80, 200, 160, 40 (top-left,
  top-right, bottom-left, bottom-right), matching the avmdec and dav2d raw outputs
- **AND** the decoded-frame hash is the pinned
  `c54ed4e996841e2178e74033d765dda1e1127d5d89c3012be3266c3e24a7fd28`

#### Scenario: Non-first block predicts from reconstructed neighbours
- **WHEN** a non-first leaf block has an in-frame above or left neighbour already
  reconstructed in the workspace
- **THEN** its § 7.13.2 DC prediction is the average of the available neighbour
  edge samples rather than the no-neighbour `128` fallback

#### Scenario: Single-block fixtures are unchanged through the unified driver
- **WHEN** `splot decode` is given the single-block `syn-flat-intra-64x64-q80.ivf`
  or `syn-cos-intra-64x64-q180.ivf` fixtures
- **THEN** they decode through the multi-block driver as a one-leaf tree
- **AND** their pinned decoded-frame hashes are unchanged
