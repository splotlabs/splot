## ADDED Requirements

### Requirement: Ordinary branch tx size dimensions support row
The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-DIMENSIONS` as a distinct crate-private
row named `coeff-ordinary-branch-tx-size-dimensions`. The row SHALL mark only
the ordinary branch `txSz` to generated width/height dimension handoff as
partial support, SHALL cite AV2 v1.0.0 sections 5.20.7.27 and 9.2, and SHALL
keep `Tx_Size_Sqr`, `txSzCtx`, `Adjusted_Tx_Size`, `compute_tx_type`, scan
derivation, runtime `coeffs()` wiring, dequantization, reconstruction, output,
reference refresh, and broad `decode_block()` / `decode_tile()` support honestly
unsupported or partial.

#### Scenario: Matrix records narrow support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-ordinary-branch-tx-size-dimensions` appears with Feature ID
  `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-DIMENSIONS`
- **AND** it names focused unit tests proving nonzero branch equivalence,
  all-zero preservation, and invalid-`txSz` fail-atomic behavior
- **AND** it does not claim `txSzCtx` derivation, adjusted transform-size
  derivation, runtime coefficient loop integration, scan order derivation,
  transform-type computation, dequantization, reconstruction, output, reference
  refresh, AVM/dav2d evidence, or full decoder conformance

#### Scenario: Generated support docs include the row
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** the new row is included in the generated status and coverage
  documents with status `partial`
