## ADDED Requirements

### Requirement: Track Runtime Coefficient Frame-Entry Handoff

The decoder support matrix and decoder conformance coverage tooling SHALL track
`DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF` as a partial runtime all-zero
coefficient handoff linked to the existing frame-facts coefficient wrapper.

#### Scenario: support matrix includes the row

- **WHEN** decoder support status is generated
- **THEN** it includes a partial row for
  `DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF`
- **AND** the row states that only the minimal runtime all-zero coefficient
  entry uses the top frame-facts wrapper
- **AND** it does not claim broad nonzero `coeffs()`, dequantization,
  reconstruction, output expansion, reference refresh, or full decoder
  conformance.

#### Scenario: conformance coverage maps the row

- **WHEN** decoder conformance coverage is checked
- **THEN** the tile-group syntax and decoder support coverage groups reference
  `DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF`.
