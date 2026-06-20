## ADDED Requirements

### Requirement: General intra luma coefficient decode support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-LUMA-COEFFS` as a
distinct partial `splot-decode` row named `general-intra-luma-coeffs`. The row
SHALL cite AV2 § 5.20.7.27 and § 8.3.2, SHALL record unit tests for the
`txb_skip` transform-size context derivation and the CLI test proving the
general intra fixture decodes its luma coefficients and reaches the chroma step,
and SHALL keep chroma coefficient decode, dequantization, inverse transform,
residual add, reconstruction, output, tile context-line commit, and public APIs
out of scope.

#### Scenario: Matrix records narrow luma coefficient support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-luma-coeffs` appears with Feature ID
  `DECODE-GENERAL-INTRA-LUMA-COEFFS`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim chroma coefficient decode, dequantization, inverse
  transform, residual add, reconstruction, or output

#### Scenario: Coverage tracks the new luma coefficient decode
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group and payload syntax coverage includes row
  `general-intra-luma-coeffs` and Feature ID `DECODE-GENERAL-INTRA-LUMA-COEFFS`
- **AND** broader tile payload coverage remains partial
