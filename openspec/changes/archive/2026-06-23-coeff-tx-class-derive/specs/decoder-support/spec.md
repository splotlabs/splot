## ADDED Requirements

### Requirement: Coefficient transform-class derivation support row
The decoder support model SHALL track `DECODE-COEFF-TX-CLASS-DERIVE` as a
distinct crate-private row named `coeff-tx-class-derive`. The row SHALL mark
only the decode-local `PlaneTxType -> txClass` mapping and max-level handoff as
partial support, SHALL cite AV2 v1.0.0 § 5.20.7.27 and § 8.3.2, and SHALL keep
`compute_tx_type`, scan derivation, runtime `coeffs()` wiring, dequantization,
reconstruction, output, reference refresh, and broad `decode_block()` /
`decode_tile()` support honestly unsupported or partial.

#### Scenario: Matrix records narrow support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-tx-class-derive` appears with Feature ID
  `DECODE-COEFF-TX-CLASS-DERIVE`
- **AND** it names focused unit tests proving the mapping and max-level handoff
  equivalence
- **AND** it does not claim runtime transform-type computation, scan order
  derivation, nonzero coefficient runtime integration, dequantization,
  reconstruction, output, reference refresh, AVM/dav2d evidence, or full decoder
  conformance

#### Scenario: Generated support docs include the row
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** the new row is included in the generated status and coverage
  documents with status `partial`
