## ADDED Requirements

### Requirement: Track runtime coefficient tx-size geometry handoff

The decoder support matrix and decoder conformance coverage tooling SHALL track
`DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF` as a partial runtime all-zero
coefficient handoff that derives the minimal luma and V `txSz` wrapper inputs
from traced transform geometry and generated AV2 section 9.2 transform-size
tables.

#### Scenario: support matrix includes the row

- **WHEN** decoder support status is generated
- **THEN** it includes a partial row for
  `DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF`
- **AND** the row states that only the minimal runtime all-zero coefficient
  transform-size geometry handoff is implemented
- **AND** it does not claim broad nonzero `coeffs()`, dequantization,
  reconstruction, output expansion, reference refresh, or full decoder
  conformance.

#### Scenario: conformance coverage maps the row

- **WHEN** decoder conformance coverage is checked
- **THEN** the tile-group syntax and decoder support coverage groups reference
  `DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF`.
