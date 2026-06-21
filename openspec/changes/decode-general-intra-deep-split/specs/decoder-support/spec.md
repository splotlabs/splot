## ADDED Requirements

### Requirement: General intra deeper square SPLIT decode support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-DEEP-SPLIT` as a
distinct partial `splot-decode` row named `general-intra-deep-split`. The row
SHALL cite AV2 § 5.20.2.3, § 5.20.3.1, § 5.20.4.1, and § 7.13.2.4, SHALL record
the deeper-split oracle test plus the single-level quad regression test, SHALL
carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the two-level
partition-tree fixture, and SHALL keep the § 5.20.2.3 `BlockDecoded` flag state,
non-DC / rectangular-leaf sub-32x32 partitions, non-64x64 frames, inter
prediction, in-loop filters, and public APIs out of scope.

#### Scenario: Matrix records narrow deeper-split support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-deep-split` appears with Feature ID
  `DECODE-GENERAL-INTRA-DEEP-SPLIT`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim the § 5.20.2.3 `BlockDecoded` flag state, non-DC /
  rectangular-leaf sub-32x32 partitions, or non-64x64 frames

#### Scenario: Coverage tracks the new deeper-split decode
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group and payload syntax coverage includes row
  `general-intra-deep-split` and Feature ID `DECODE-GENERAL-INTRA-DEEP-SPLIT`
- **AND** broader tile payload coverage remains partial
