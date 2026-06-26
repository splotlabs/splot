## ADDED Requirements

### Requirement: 10-bit general intra DC decode support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-10BIT` as a distinct
partial `splot-decode` row named `general-intra-10bit`. The row SHALL cite AV2
§ 6.4.1, § 7.13.2, § 7.14.3, § 7.14.4, § 7.15.4, and § 8.2.4, SHALL record the
10-bit DC_PRED-luma + DC-chroma square-leaf oracle tests (flat single-64x64 DC,
single-64x64 eob > 1 AC residual, and multi-64x64-superblock DC), SHALL carry the
reciprocal LOCAL-REFERENCE-EVIDENCE pointers for those 10-bit fixtures, and SHALL
keep 10-bit non-DC prediction, 10-bit rectangular (non-square) partition-leaf
reconstruction, 10-bit inter prediction / reference retention, in-loop filters,
and public APIs out of scope.

#### Scenario: Matrix records 10-bit DC square-leaf support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-10bit` appears with Feature ID
  `DECODE-GENERAL-INTRA-10BIT`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim 10-bit non-DC prediction, 10-bit rectangular
  (non-square) partition-leaf reconstruction, or 10-bit inter / reference
  retention

#### Scenario: Coverage tracks the new 10-bit DC decode
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group and payload syntax coverage includes row
  `general-intra-10bit` and Feature ID `DECODE-GENERAL-INTRA-10BIT`
- **AND** broader tile payload coverage remains partial
