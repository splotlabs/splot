## ADDED Requirements

### Requirement: General intra frame frontier support row
The decoder support model SHALL track `DECODE-GENERAL-INTRA-FRAME-FRONTIER` as a
distinct partial `splot-decode` row named `general-intra-frame-frontier`. The
row SHALL cite AV2 § 5.18.2, § 5.20.1, § 5.20.3.1, and § 5.20.3.2, SHALL record
focused tests for reaching the partition frontier on the committed
`syn-flat-intra-64x64-q80.ivf` fixture and for the frozen-hash regression guard,
and SHALL keep arbitrary intra mode decode, coefficient symbol reads, `Quant`
writes, dequantization, inverse transform, residual add, reconstruction, output,
reference refresh, and public APIs out of scope.

#### Scenario: Matrix records narrow general intra frontier support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `general-intra-frame-frontier` appears with Feature ID
  `DECODE-GENERAL-INTRA-FRAME-FRONTIER`
- **AND** it is marked partial rather than supported for full runtime decode
- **AND** it does not claim coefficient decode, dequantization, inverse
  transform, residual add, reconstruction, output, or reference refresh

#### Scenario: Coverage tracks the new general intra frontier
- **WHEN** decoder conformance coverage is generated
- **THEN** the tile group and payload syntax coverage includes row
  `general-intra-frame-frontier` and Feature ID
  `DECODE-GENERAL-INTRA-FRAME-FRONTIER`
- **AND** broader tile payload coverage remains partial
