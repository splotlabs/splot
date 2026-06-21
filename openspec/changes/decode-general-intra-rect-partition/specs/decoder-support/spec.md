## ADDED Requirements

### Requirement: General intra rectangular partition decode support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-RECT-PARTITION` as a
distinct partial `splot-decode` row named `general-intra-rect-partition`. The row
SHALL cite AV2 § 5.20.3.1, § 5.20.4.1, § 5.20.7.27, § 7.13.2.4, § 7.14.4,
§ 7.15.4, and § 8.2.4, SHALL record the rectangular oracle test plus the
single-level quad regression test, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointer for the rectangular partition fixture, and SHALL
keep non-DC rectangular luma / chroma prediction, the § 5.20.2.3 `BlockDecoded`
flag state, non-64x64 frames, inter prediction, in-loop filters, and public APIs
out of scope.

#### Scenario: Matrix records narrow rectangular-partition support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-rect-partition` appears with Feature ID
  `DECODE-GENERAL-INTRA-RECT-PARTITION`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim non-DC rectangular prediction, the § 5.20.2.3
  `BlockDecoded` flag state, or non-64x64 frames

#### Scenario: Coverage tracks the new rectangular-partition decode
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group and payload syntax coverage includes row
  `general-intra-rect-partition` and Feature ID `DECODE-GENERAL-INTRA-RECT-PARTITION`
- **AND** broader tile payload coverage remains partial
