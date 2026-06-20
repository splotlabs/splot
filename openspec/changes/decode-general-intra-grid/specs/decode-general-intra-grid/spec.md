## ADDED Requirements

### Requirement: General intra full 2-D superblock grid decode
The decoder SHALL decode single-tile 8-bit 4:2:0 intra key frames whose width and
height are positive multiples of 64 and which form a full 2-D grid of 64x64
superblocks, on the general intra path, iterating every superblock in the tile's
MI range in raster order per AV2 § 5.20.2.1 `decode_tile()`. For a full-superblock
§ 7.13.2.13 `SMOOTH_PRED` chroma block whose above-right neighbour is already
decoded (a non-rightmost row>0 superblock), the decoder SHALL set the § 7.13.2.1
top-right sentinel `AboveRow[w]` to the real reconstructed
`CurrFrame[plane][y - 1][Min(aboveLimit, x + w)]` sample, with
`aboveLimit = Min(maxX, x + w + 4 * num4AboveRight - 1)` and `num4AboveRight`
derived faithfully to AV2 § 5.20.7.25 `count_top_right_avail` over the § 5.20.2.3
`BlockDecoded` state; when the above-right is not decoded (`num4AboveRight == 0`)
or the block touches the chroma frame right edge, the sentinel SHALL be the
clamped last in-block above sample. The bottom-left sentinel `LeftCol[h]` SHALL
remain the clamped last in-block left sample (in raster decode order
`num4BelowLeft == 0`). It SHALL keep the frozen `base_q_idx == 255` minimal hash
tier's strict 64x64 requirement unchanged. It SHALL reject — with a structured
`decode/unsupported-feature` diagnostic — a frame whose width or height is not a
positive multiple of 64, a directional luma mode, SMOOTH chroma on a
sub-partitioned (non-full-superblock) block, other non-DC chroma modes, multiple
tiles, inter prediction, and in-loop filters, and SHALL NOT invoke AVM or dav2d.

#### Scenario: 2-D grid 128x128 frame decodes to the oracle
- **WHEN** `splot decode` is given the committed full 2-D grid intra key frame
  `syn-grid-intra-128x128-q80.ivf`
- **THEN** the general intra path iterates the four 64x64 superblocks, and the
  bottom-left superblock's `SMOOTH_PRED` chroma reads the real reconstructed
  above-right neighbour (the top-right superblock), and succeeds
- **AND** the luma is uniform 100, chroma is distinct flat per quadrant (U
  top-left 110 / top-right 200 / bottom-right 130) except the SMOOTH bottom-left
  superblock, matching the avmdec and dav2d raw outputs byte-for-byte (md5
  `dd2fa84f802c72fea4472b3af87104f1`)
- **AND** the decoded-frame hash is the pinned
  `42bd99faae1ac0acb15c3e24fbededd8fc670612d08987bebb8942de5f4f4874`

#### Scenario: The above-right sentinel is actually read
- **WHEN** the bottom-left superblock's `SMOOTH_PRED` chroma block is
  reconstructed
- **THEN** its top-right corner is pulled toward the decoded above-right
  neighbour value (200), not the edge-clamped own-top value (110), so the
  edge-clamp (repeat-last) sentinel would mismatch the oracle while the real
  above-right read is bit-exact

#### Scenario: Existing single-row/column frames still decode bit-exact
- **WHEN** `splot decode` is given the committed 64x64, 128x64, and 64x128 general
  intra fixtures
- **THEN** each reconstructs to its previously pinned decoded-frame hash,
  unchanged by the 2-D grid generalization
