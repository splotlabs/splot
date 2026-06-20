## ADDED Requirements

### Requirement: General intra multi-superblock-row (grid) decode
The decoder SHALL decode single-tile 8-bit 4:2:0 intra key frames whose width and
height are both positive multiples of 64 — a grid of 64x64 superblocks — on the
general intra path, iterating every superblock in the tile's MI range in raster
order (multiple rows and columns) per AV2 § 5.20.2.1 `decode_tile()` with
`clear_left_context()` at the start of each superblock row, so that a second-row
superblock DC-predicts its luma from the already-reconstructed first-row above
neighbour. It SHALL reconstruct a full-superblock (64x64) block's § 7.13.2.13
`SMOOTH_PRED` chroma correctly at any row (for a full superblock
`clear_block_decoded_flags` (§ 5.20.2) zeroes the above-right region and the
below-left is decoded later, so the § 7.13.2.1 sentinels are the edge-clamped last
neighbour sample). It SHALL keep the frozen `base_q_idx == 255` minimal hash
tier's strict 64x64 requirement unchanged. It SHALL reject — with a structured
`decode/unsupported-feature` diagnostic — a frame whose width or height is not a
positive multiple of 64, a directional luma mode, SMOOTH chroma on a
sub-partitioned (non-full-superblock) block, other non-DC chroma modes, multiple
tiles, inter prediction, and in-loop filters, and SHALL NOT invoke AVM or dav2d.

#### Scenario: Uniform 128x128 grid frame decodes to the oracle
- **WHEN** `splot decode` is given the committed multi-superblock-row intra key
  frame `syn-uniform-intra-128x128-q80.ivf`
- **THEN** the general intra path iterates the 2x2 grid of 64x64 superblocks,
  with the second-row superblocks DC-predicting from the reconstructed first-row
  neighbours, and succeeds
- **AND** every reconstructed luma sample is 100 and chroma U=120 / V=130,
  matching the avmdec and dav2d raw outputs byte-for-byte (md5
  `df1bc678abe0e206769ac9bbd7b98d7f`)
- **AND** the decoded-frame hash is the pinned
  `1bfd079174c7494086aab6d37f61dec25e850d42767f6adc3969e2969384d6eb`

#### Scenario: Existing single-row frames still decode bit-exact
- **WHEN** `splot decode` is given the committed 64x64 and 128x64 general intra
  fixtures
- **THEN** each reconstructs to its previously pinned decoded-frame hash,
  unchanged by the grid generalization
