## ADDED Requirements

### Requirement: General intra full-superblock SMOOTH_H luma cross-superblock above-right
The decoder SHALL reconstruct a full-superblock (`n4w == 16`) § 7.13.2.13
`SMOOTH_H_PRED` luma block on the general intra path at ANY superblock position in
a 2-D grid, including a superblock at row > 0 (`frontier.r > 0`). For a row > 0
block the decoder SHALL build the § 7.13.2.1 edges using `haveLeft`/`haveAbove`
per position and SHALL resolve the top-right sentinel `AboveRow[w]` to the real
reconstructed above-right sample `CurrFrame[0][y - 1][Min(aboveLimit, x + w)]`
when an already-decoded above-right superblock is in frame (`num4AboveRight > 0`,
derived faithfully to AV2 § 5.20.7.25 `count_top_right_avail` over the
§ 5.20.2.3 `BlockDecoded` state) — the same edge-building and above-right
resolution the SMOOTH chroma 2-D grid path uses for the chroma (`sub_x == 1`)
plane, now applied to the luma (`sub_x == 0`) above-right VALUE that the
`SMOOTH_H_PRED` § 7.13.2.13 `predH2` reads. It SHALL then run the § 7.13.2.13
SMOOTH_H predictor (linear interpolation, no `enable_intra_edge_filter` / IDIF /
upsample edge synthesis) and add the § 5.20.7.27 residual. The decoder SHALL
admit this only for a full-superblock (`n4w == 16`) block; it SHALL reject — with
a structured `decode/unsupported-feature` diagnostic — a `SMOOTH_H_PRED`
sub-partitioned (SPLIT-child) block whose cross-superblock above-right is read at
superblock-relative row 0, a SMOOTH_V below-left sub-block sentinel, a SMOOTH
chroma sub-block, a neighbour-having directional (D135) or PAETH luma block,
multiple tiles, inter prediction, and in-loop filters, and SHALL NOT invoke AVM
or dav2d.

#### Scenario: A row > 0 SMOOTH_H luma superblock decodes to the oracle
- **WHEN** `splot decode` is given the committed 2-D grid intra key frame
  `syn-shgrid-intra-128x128-q80.ivf`, whose bottom-left (row > 0, non-rightmost)
  superblock codes `SMOOTH_H_PRED` luma over a horizontal gradient
- **THEN** the general intra path iterates the four 64x64 superblocks, and the
  bottom-left SMOOTH_H luma superblock reads the real reconstructed
  cross-superblock above-right sentinel (`num4AboveRight == 16`), and succeeds
- **AND** the decoded output matches the avmdec and dav2d raw outputs
  byte-for-byte (md5 `fe420ce870c13a8055aa83fd5aa64740`)
- **AND** the decoded-frame hash is the pinned
  `d1ce39cc3d79f5c46fdea67ad57ec4edd5dfed088ee39fd7029fda1bbb11e0e8`

#### Scenario: The SMOOTH_H block reads the real above-right, not the clamp
- **WHEN** the bottom-left (row > 0) SMOOTH_H luma superblock is reconstructed
- **THEN** its § 7.13.2.1 top-right sentinel `AboveRow[w]` is the real
  reconstructed bottom row of the already-decoded diagonally-above-right superblock
  (the distinct flat value 200), NOT the edge-clamp candidate (100, the
  bottom-left's own above sample), so the reconstructed rightmost column blends
  toward 200

#### Scenario: A SMOOTH_H SPLIT-child cross-superblock above-right is still deferred
- **WHEN** a `SMOOTH_H_PRED` sub-partitioned (SPLIT-child) luma block at
  superblock-relative row 0 would read its above-right sentinel from a
  cross-superblock (row > 0) decoded neighbour
- **THEN** the decoder emits a structured `decode/unsupported-feature` diagnostic
  (`general_intra_smooth_h_above_right_unverified`), since only the full-superblock
  SMOOTH_H row > 0 above-right is oracle-verified

#### Scenario: A SMOOTH chroma sub-block is still rejected
- **WHEN** `splot decode` is given a frame whose 64x64 superblock SPLITs and the
  encoder codes a SMOOTH chroma sub-block (e.g. the committed negative
  `syn-svsplit-intra-64x64-q140.ivf`)
- **THEN** the decoder emits a structured `decode/unsupported-feature` diagnostic
  (`general_intra_smooth_chroma_subblock`), unchanged by lifting the full-superblock
  SMOOTH_H luma gate

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed 64x64, 128x64, 64x128, 192x128,
  and 128x128 general intra fixtures
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by lifting the full-superblock SMOOTH_H row > 0 above-right gate
