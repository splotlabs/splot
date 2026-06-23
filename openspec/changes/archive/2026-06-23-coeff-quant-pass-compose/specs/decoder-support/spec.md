## ADDED Requirements

### Requirement: Coefficient quant-pass composition support row
The decoder support model SHALL track `DECODE-COEFF-QUANT-PASS-COMPOSE` as a
distinct crate-private row named `coeff-quant-pass-compose`. The row SHALL mark
only ordinary non-FSC `read_quant` to `Quant[]` composition as partial
coefficient-loop support, SHALL cite focused self-contained tests, and SHALL
keep runtime `coeffs()` integration, dequantization, reconstruction, broad
`decode_tile()`, and byte-identical decode honestly partial or unsupported.

#### Scenario: Matrix records narrow composition support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-quant-pass-compose` appears with Feature ID
  `DECODE-COEFF-QUANT-PASS-COMPOSE`
- **AND** it cites AV2 § 5.20.7.27 and § 5.20.7.28 as syntax evidence
- **AND** it names tests for positive composition, hidden-parity composition,
  TCQ composition, and no-consumption plus no-mutation caller-fact failures
- **AND** it does not claim runtime coefficient-loop execution, selector or
  scan-table derivation, dequantization, inverse transform, residual add,
  reconstruction, reference refresh, AVM/dav2d evidence, public APIs, or full
  decoder conformance

#### Scenario: Generated docs remain honest
- **WHEN** feature status, spec coverage, decoder support, and decoder
  conformance coverage status documents are regenerated
- **THEN** the new row remains partial until a later runtime `coeffs()`
  integration change proves reachable decode behavior
