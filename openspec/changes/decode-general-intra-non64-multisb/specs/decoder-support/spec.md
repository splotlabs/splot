## ADDED Requirements

### Requirement: General intra multi-superblock non-64x64 support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-NON64-MULTISB` as a
distinct partial `splot-decode` row named `general-intra-non64-multisb`. The row
SHALL cite AV2 § 5.20.2.1, § 5.20.3.1, § 5.20.5.3, § 7.13.2.1, and § 7.13.2.13,
SHALL record the two-superblock oracle test, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointer for the 128x64 fixture, and SHALL keep partial
frames (non-multiple-of-64 sizes), non-DC/non-SMOOTH chroma, multiple tiles,
inter prediction, and in-loop filters out of scope.

#### Scenario: Matrix records narrow multi-superblock support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-non64-multisb` appears with Feature ID
  `DECODE-GENERAL-INTRA-NON64-MULTISB`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim partial-frame (non-multiple-of-64) sizes, multiple
  tiles, inter prediction, or in-loop filters
