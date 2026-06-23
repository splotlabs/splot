## ADDED Requirements

### Requirement: General intra full 2-D superblock grid support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-GRID` as a distinct
partial `splot-decode` row named `general-intra-grid`. The row SHALL cite AV2
§ 5.18.3, § 5.20.2.3, § 5.20.7.25, § 7.13.2.1, and § 7.13.2.13, SHALL record the
2-D grid 128x128 oracle test, the `num4AboveRight` derivation test, and the
workspace `reconstructed_sample` test, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointer for the 128x128 fixture, and SHALL keep
directional luma, SMOOTH chroma on sub-partitioned blocks, partial frames, and
multiple tiles out of scope.

#### Scenario: Matrix records the full 2-D superblock grid support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-grid` appears with Feature ID
  `DECODE-GENERAL-INTRA-GRID`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim directional luma, sub-partitioned SMOOTH chroma, or
  multiple tiles
