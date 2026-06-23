## ADDED Requirements

### Requirement: General intra block mode-info support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-BLOCK-MODES` as a
distinct partial `splot-decode` row named `general-intra-block-modes`. The row
SHALL cite AV2 § 5.20.5.3 and § 8.3.2, SHALL record a unit test that
reconstructs `DC_PRED` luma and a valid chroma mode in spec order and the CLI
test that proves the general intra fixture decodes its modes and reaches the
residual step, and SHALL keep typed `UVMode` reconstruction, coefficient
symbol reads, `Quant` writes, dequantization, inverse transform, residual add,
reconstruction, output, and public APIs out of scope.

#### Scenario: Matrix records narrow mode-info support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-block-modes` appears with Feature ID
  `DECODE-GENERAL-INTRA-BLOCK-MODES`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim coefficient decode, dequantization, inverse
  transform, residual add, reconstruction, or output

#### Scenario: Coverage tracks the new mode-info decode
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group and payload syntax coverage includes row
  `general-intra-block-modes` and Feature ID
  `DECODE-GENERAL-INTRA-BLOCK-MODES`
- **AND** broader tile payload coverage remains partial
