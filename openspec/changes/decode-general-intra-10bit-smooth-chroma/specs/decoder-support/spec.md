## ADDED Requirements

### Requirement: 10-bit general intra DC-luma + SMOOTH-chroma decode support row
The decoder support model SHALL track
`DECODE-GENERAL-INTRA-10BIT-SMOOTH-CHROMA` as a distinct partial `splot-decode`
row named `general-intra-10bit-smooth-chroma`. The row SHALL cite AV2 § 6.4.1,
§ 7.13.2.1, and § 7.13.2.13, SHALL record the 10-bit DC-luma + top-left
no-neighbour SMOOTH-chroma oracle test, SHALL carry the reciprocal
LOCAL-REFERENCE-EVIDENCE pointer for the
`syn-smchroma-intra-64x64-10bit-q160.ivf` fixture, and SHALL keep
neighbour-having SMOOTH chroma, 10-bit non-DC luma, 10-bit non-64x64
partition-leaf reconstruction, 10-bit inter prediction / reference retention,
in-loop filters, and public APIs out of scope.

#### Scenario: Matrix records 10-bit DC-luma + SMOOTH-chroma support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-10bit-smooth-chroma` appears with Feature ID
  `DECODE-GENERAL-INTRA-10BIT-SMOOTH-CHROMA`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim neighbour-having SMOOTH chroma, 10-bit non-DC luma,
  or 10-bit non-64x64 partition-leaf reconstruction

#### Scenario: Coverage tracks the new 10-bit SMOOTH-chroma decode
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group and payload syntax coverage includes row
  `general-intra-10bit-smooth-chroma` and Feature ID
  `DECODE-GENERAL-INTRA-10BIT-SMOOTH-CHROMA`
- **AND** broader tile payload coverage remains partial
