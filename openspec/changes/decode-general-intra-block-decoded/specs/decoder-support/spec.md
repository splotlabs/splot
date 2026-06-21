## ADDED Requirements

### Requirement: General intra per-block BlockDecoded support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-BLOCK-DECODED` as a
distinct partial `splot-decode` row named `general-intra-block-decoded`. The row
SHALL cite AV2 § 5.20.2.3, § 5.20.7.25, § 7.13.2.1, and § 7.13.2.13, SHALL record
the SMOOTH_H split sub-block above-right oracle test, the SMOOTH chroma
split-sub-block reject test, and the `BlockDecoded` grid unit test, SHALL carry
the reciprocal LOCAL-REFERENCE-EVIDENCE pointers for the positive and negative
fixtures, and SHALL keep SMOOTH_V below-left sub-block sentinels, SMOOTH chroma
sub-blocks, directional sub-blocks, non-DCTONLY-size non-DC sub-blocks, non-64x64
runtime beyond the existing subset, inter prediction, in-loop filters, and public
APIs out of scope.

#### Scenario: Matrix records narrow BlockDecoded support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-block-decoded` appears with Feature ID
  `DECODE-GENERAL-INTRA-BLOCK-DECODED`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim SMOOTH_V below-left sub-block sentinels, SMOOTH
  chroma sub-blocks, or directional sub-blocks

#### Scenario: Coverage tracks the new BlockDecoded decode
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group and payload syntax coverage includes row
  `general-intra-block-decoded` and Feature ID
  `DECODE-GENERAL-INTRA-BLOCK-DECODED`
- **AND** broader tile payload coverage remains partial
