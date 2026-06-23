## ADDED Requirements

### Requirement: Ordinary branch geometry handoff support row
The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-BRANCH-GEOMETRY-HANDOFF` as a distinct crate-private row
named `coeff-ordinary-branch-geometry-handoff`. The row SHALL mark only the
ordinary branch block-start geometry to state-context geometry handoff as partial
support, SHALL cite AV2 v1.0.0 section 5.20.7.27, and SHALL keep raw
`startX`/`startY`/`txSz` derivation, `compute_tx_type`, scan derivation, runtime
`coeffs()` wiring, dequantization, reconstruction, output, reference refresh,
and broad `decode_block()` / `decode_tile()` support honestly unsupported or
partial.

#### Scenario: Matrix records narrow support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-ordinary-branch-geometry-handoff` appears with Feature ID
  `DECODE-COEFF-ORDINARY-BRANCH-GEOMETRY-HANDOFF`
- **AND** it names focused unit tests proving nonzero branch equivalence and
  all-zero preservation
- **AND** it does not claim raw transform-block geometry derivation, runtime
  coefficient loop integration, scan order derivation, transform-type
  computation, dequantization, reconstruction, output, reference refresh,
  AVM/dav2d evidence, or full decoder conformance

#### Scenario: Generated support docs include the row
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** the new row is included in the generated status and coverage
  documents with status `partial`
