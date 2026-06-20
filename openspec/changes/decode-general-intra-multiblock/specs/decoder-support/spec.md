## ADDED Requirements

### Requirement: General intra multi-block decode support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-MULTIBLOCK` as a
distinct partial `splot-decode` row named `general-intra-multiblock`. The row
SHALL cite AV2 § 5.20.3.1, § 5.20.4.1, § 5.20.5.3, § 5.20.7.27, § 7.13.2,
§ 8.2.4, and § 8.3.2, SHALL record the multi-block oracle test plus a
single-block regression test, SHALL carry the reciprocal LOCAL-REFERENCE-EVIDENCE
pointer for the four-quadrant fixture, and SHALL keep non-DC modes,
rectangular-leaf partitions, multiple tiles, non-64x64 frames, and public APIs
out of scope.

#### Scenario: Matrix records narrow multi-block support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-multiblock` appears with Feature ID
  `DECODE-GENERAL-INTRA-MULTIBLOCK`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim non-DC modes, rectangular-leaf partitions, multiple
  tiles, or non-64x64 frames

#### Scenario: Coverage tracks the new multi-block decode
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group and payload syntax coverage includes row
  `general-intra-multiblock` and Feature ID `DECODE-GENERAL-INTRA-MULTIBLOCK`
- **AND** broader tile payload coverage remains partial
