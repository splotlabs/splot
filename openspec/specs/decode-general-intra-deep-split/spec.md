# decode-general-intra-deep-split Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `decode-general-intra-deep-split`.

## Requirements
### Requirement: General intra deeper (sub-32x32) square SPLIT decode
The decoder SHALL decode a two-level square SPLIT partition tree on the general
intra path: a 64x64 8-bit 4:2:0 intra key frame whose 64x64 superblock SPLITs
into four 32x32 quadrants and whose one 32x32 quadrant SPLITs AGAIN into four
square 16x16 DC_PRED leaves (the other three quadrants staying 32x32 DC_PRED).
Each sub-32x32 16x16 leaf's § 7.13.2.10 DC prediction SHALL read its in-frame
left column / above row from the persistent frame workspace, so the leaf
DC-predicts from its already-reconstructed sibling 16x16 neighbour inside the
parent 32x32 sub-block, in § 5.20.3.1 decode (DFS) order. It SHALL validate
§ 8.2.4 `exit_symbol()` after the whole tile. It SHALL reject a non-DC or
non-square (rectangular leaf) sub-32x32 partition with a structured
`decode/unsupported-feature` diagnostic. It SHALL NOT require the § 5.20.2.3
`BlockDecoded` flag state (the DC predictor never reads the § 7.13.2.1
above-right / below-left sentinels), and SHALL NOT handle non-64x64 frames,
inter prediction, in-loop filters, or invoke AVM or dav2d.

#### Scenario: Two-level square split decodes to the oracle
- **WHEN** `splot decode` is given the committed two-level partition-tree intra
  key frame `syn-deep-intra-64x64-q120.ivf`
- **THEN** the general intra path walks the partition tree into four square 16x16
  DC_PRED leaves in the top-left 32x32 plus three 32x32 DC_PRED quadrants,
  decoding and reconstructing each in decode order, and succeeds
- **AND** the reconstructed top-left 16x16 leaf centres are 240, 21, 21, 240
  (top-left, top-right, bottom-left, bottom-right) and the other three 32x32
  quadrant centres are 130, 70, 200, matching the avmdec and dav2d raw outputs
- **AND** the decoded-frame hash is the pinned
  `73123e51c66787b59fb6b93a6221e9d78a550c6e0d1c4e0c1adfd21a41ed39ab`

#### Scenario: Sub-32x32 16x16 leaf predicts from a reconstructed sibling
- **WHEN** a 16x16 leaf inside a SPLIT 32x32 has an in-frame above or left 16x16
  sibling already reconstructed in the workspace
- **THEN** its § 7.13.2.10 DC prediction is the average of the available sibling
  neighbour edge samples rather than the no-neighbour `128` fallback

#### Scenario: Existing general intra fixtures are unchanged
- **WHEN** `splot decode` is given the existing general intra fixtures
  (`syn-flat-intra-64x64-q80.ivf`, `syn-quad-intra-64x64-q80.ivf`, and the
  remaining committed vectors)
- **THEN** they decode bit-exactly to their pinned decoded-frame hashes
