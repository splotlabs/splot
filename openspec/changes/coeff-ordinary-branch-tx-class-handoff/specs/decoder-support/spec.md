## ADDED Requirements

### Requirement: Ordinary branch transform-class handoff support row
The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-BRANCH-TX-CLASS-HANDOFF` as a distinct crate-private row
named `coeff-ordinary-branch-tx-class-handoff`. The row SHALL mark only the
ordinary branch `PlaneTxType -> txClass` handoff as partial support, SHALL cite
AV2 v1.0.0 section 5.20.7.27 and section 8.3.2, and SHALL keep
`compute_tx_type`, scan derivation, runtime `coeffs()` wiring, dequantization,
reconstruction, output, reference refresh, and broad `decode_block()` /
`decode_tile()` support honestly unsupported or partial.

#### Scenario: Matrix records narrow support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-ordinary-branch-tx-class-handoff` appears with Feature ID
  `DECODE-COEFF-ORDINARY-BRANCH-TX-CLASS-HANDOFF`
- **AND** it names focused unit tests proving nonzero branch equivalence and
  all-zero preservation
- **AND** it does not claim runtime transform-type computation, scan order
  derivation, nonzero coefficient runtime integration, dequantization,
  reconstruction, output, reference refresh, AVM/dav2d evidence, or full decoder
  conformance

#### Scenario: Generated support docs include the row
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** the new row is included in the generated status and coverage
  documents with status `partial`
