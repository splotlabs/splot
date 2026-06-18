## ADDED Requirements

### Requirement: Coefficient read-quant syntax support row
The decoder support model SHALL track `DECODE-COEFF-READ-QUANT-SYNTAX` as a
distinct crate-private row named `coeff-read-quant-syntax`. The row SHALL mark
only AV2 § 5.20.7.28 `read_quant` literal syntax parsing as partial
coefficient-loop support, SHALL cite focused self-contained tests, and SHALL
keep runtime `coeffs()` integration, dequantization, reconstruction, broad
`decode_tile()`, and byte-identical decode honestly partial or unsupported.

#### Scenario: Matrix records narrow parser support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-read-quant-syntax` appears with Feature ID
  `DECODE-COEFF-READ-QUANT-SYNTAX`
- **AND** it cites AV2 § 5.20.7.28 as syntax evidence
- **AND** it names tests for the threshold skip path, finite q-length path,
  Golomb extension path, hidden DC and TCQ facts, malformed-prefix guards, and
  overflow failures
- **AND** it does not claim runtime coefficient-loop execution, nonzero
  `Quant[]` production, dequantization, inverse transform, residual add,
  reconstruction, reference refresh, AVM/dav2d evidence, public APIs, or full
  decoder conformance

#### Scenario: Generated docs remain honest
- **WHEN** feature status, spec coverage, decoder support, and decoder
  conformance coverage status documents are regenerated
- **THEN** the new row remains partial until a later runtime `coeffs()`
  integration change proves reachable decode behavior
