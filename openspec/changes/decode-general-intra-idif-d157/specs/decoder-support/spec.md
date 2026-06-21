## ADDED Requirements

### Requirement: General intra D157 luma IDIF support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-IDIF-D157` as a distinct
partial `splot-decode` row named `general-intra-idif-d157`. The row SHALL cite AV2
§ 5.20.5.3, § 5.20.7.27, § 7.13.2.1, § 7.13.2.7, § 7.13.2.8, § 8.2.4, and § 9.2,
SHALL record the D157 neighbour-having luma IDIF + follow chroma oracle test and the
`splot-recon` IDIF 4-tap unit tests, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointer for the `syn-d157-intra-128x64-q80.ivf` fixture,
and SHALL keep the top-left / first-column / sub-superblock / row>0 D157 positions,
the other middle angle D113, the one-sided angles D45/D67/D203, non-zero angle
deltas, the directional-neighbour (`ctx != 0`) escape reorder, non-64x64 frames, and
multiple tiles out of scope.

#### Scenario: Matrix records the D157 luma IDIF support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** row `general-intra-idif-d157` appears with Feature ID
  `DECODE-GENERAL-INTRA-IDIF-D157`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim a top-left / first-column / row>0 D157 position, the
  D113 or one-sided angles, non-zero angle deltas, or multiple tiles
