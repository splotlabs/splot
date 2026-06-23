## ADDED Requirements

### Requirement: General intra full-superblock SMOOTH_H luma above-right support row
The decoder support model SHALL track
`DECODE-GENERAL-INTRA-GRID-SMOOTH-H-ABOVE-RIGHT` as a distinct partial
`splot-decode` row named `general-intra-grid-smooth-h-above-right`. The row SHALL
cite AV2 § 5.20.2.3, § 5.20.5.3, § 5.20.7.25, § 5.20.7.27, § 7.13.2.1, and
§ 7.13.2.13, SHALL record the 2-D grid 128x128 row > 0 SMOOTH_H luma
cross-superblock above-right oracle test, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointer for the 128x128 fixture, and SHALL keep the
SMOOTH_H SPLIT-child cross-superblock above-right, SMOOTH_V below-left sub-block
sentinels, SMOOTH chroma sub-blocks, neighbour-having directional / PAETH luma,
and multiple tiles out of scope.

#### Scenario: Matrix records the SMOOTH_H luma above-right support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-grid-smooth-h-above-right` appears with Feature ID
  `DECODE-GENERAL-INTRA-GRID-SMOOTH-H-ABOVE-RIGHT`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim SMOOTH_H sub-partitioned cross-superblock above-right,
  SMOOTH chroma sub-blocks, neighbour-having directional luma, or multiple tiles
