## ADDED Requirements

### Requirement: General intra D113 vertical-leaning angle support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-ANGLE-D113` as a
distinct partial `splot-decode` row named `general-intra-angle-d113`. The row SHALL
cite AV2 § 5.20.5.3, § 5.20.7.27, § 7.13.2.1, § 7.13.2.7, § 7.13.2.8, and § 9.2,
SHALL record the row>0 directional D113 luma IDIF + D113-follow chroma oracle test
reading the real § 7.13.2.1 above row + left column + corner, SHALL carry the
reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the
`syn-d113-intra-128x128-q80.ivf` fixture, and SHALL keep the top-left, first-row
(`haveAbove == 0`), first-column, sub-partitioned, and non-64x64 D113 positions,
the one-sided angles D45/D67/D203, non-zero angle deltas, the directional-neighbour
(`ctx != 0`) escape reorder, and multiple tiles out of scope.

#### Scenario: Matrix records the D113 vertical-leaning angle support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-angle-d113` appears with Feature ID
  `DECODE-GENERAL-INTRA-ANGLE-D113`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim a top-left / first-row / first-column / sub-partitioned
  / non-64x64 D113 position, the one-sided angles D45/D67/D203, non-zero angle
  deltas, or multiple tiles
