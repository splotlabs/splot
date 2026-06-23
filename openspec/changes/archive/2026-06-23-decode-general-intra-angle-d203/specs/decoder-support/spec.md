## ADDED Requirements

### Requirement: General intra D203 zone-3 one-sided angle support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-ANGLE-D203` as a
distinct partial `splot-decode` row named `general-intra-angle-d203`. The row SHALL
cite AV2 § 5.20.5.3, § 5.20.7.25, § 5.20.7.27, § 7.13.2.1, § 7.13.2.7, § 7.13.2.8,
and § 9.2, SHALL record the first-superblock-row non-first-column zone-3 D203 luma
IDIF + D203-follow chroma oracle test reading the real § 7.13.2.1 left column, SHALL
carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the
`syn-d203-intra-128x64-q80.ivf` fixture, and SHALL keep the top-left, first-column
(no real left column), row>0, sub-partitioned, and non-64x64 D203 positions, the
remaining one-sided angle D67, non-zero angle deltas, the directional-neighbour
(`ctx != 0`) escape reorder, and multiple tiles out of scope.

#### Scenario: Matrix records the D203 zone-3 one-sided angle support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-angle-d203` appears with Feature ID
  `DECODE-GENERAL-INTRA-ANGLE-D203`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim a top-left / first-column / row>0 / sub-partitioned /
  non-64x64 D203 position, the remaining one-sided angle D67, non-zero angle deltas,
  or multiple tiles
