## ADDED Requirements

### Requirement: Track Coefficient Frame-Facts Handoff

The decoder support matrix and decoder conformance coverage tooling SHALL track
`DECODE-COEFF-FRAME-FACTS-HANDOFF` as a partial loaded-but-unwired
decoder-support row linked to AV2 § 5.4.8, § 5.18.2, § 5.20.7.27, § 6.4.8,
and the existing base-q `useFsc` handoff row.

#### Scenario: support matrix includes the row

- **WHEN** decoder support status is generated
- **THEN** it includes a partial row for `DECODE-COEFF-FRAME-FACTS-HANDOFF`
  describing the crate-private wrapper that derives frame/sequence facts before
  delegating to the base-q `useFsc` handoff
- **AND** it records that runtime `coeffs()` integration, full
  `compute_tx_type`, runtime block syntax traversal, dequantization,
  reconstruction, output, and reference refresh remain unsupported.

#### Scenario: conformance coverage maps the row

- **WHEN** decoder conformance coverage is checked
- **THEN** the coefficient syntax and decoder support coverage groups reference
  `DECODE-COEFF-FRAME-FACTS-HANDOFF`.
