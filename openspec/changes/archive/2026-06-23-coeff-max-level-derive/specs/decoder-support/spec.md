## ADDED Requirements

### Requirement: Coefficient maxLevel derivation support row
The decoder support model SHALL track `DECODE-COEFF-MAX-LEVEL-DERIVE` as a
distinct crate-private row named `coeff-max-level-derive`. The row SHALL mark
only ordinary non-FSC `maxLevel` derivation as partial coefficient-loop support
and SHALL keep runtime `coeffs()` integration, dequantization, reconstruction,
broad `decode_tile()`, and byte-identical decode honestly partial or
unsupported.

#### Scenario: Matrix records narrow derivation support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-max-level-derive` appears with Feature ID
  `DECODE-COEFF-MAX-LEVEL-DERIVE`
- **AND** it cites AV2 § 5.20.7.27 as syntax evidence
- **AND** it names tests for transform-class/plane low-frequency limits, hidden
  final-entry override, quant-pass input conversion, and totality
- **AND** it does not claim runtime coefficient-loop execution, selector or
  scan-table derivation, dequantization, inverse transform, residual add,
  reconstruction, reference refresh, AVM/dav2d evidence, public APIs, or full
  decoder conformance

#### Scenario: Generated docs remain honest
- **WHEN** feature status, spec coverage, decoder support, and decoder
  conformance coverage status documents are regenerated
- **THEN** the new row remains partial until a later runtime `coeffs()`
  integration change proves reachable decode behavior
