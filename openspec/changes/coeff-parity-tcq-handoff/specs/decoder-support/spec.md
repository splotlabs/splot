## ADDED Requirements

### Requirement: Track Coefficient Parity and TCQ Handoff

The decoder support matrix and decoder conformance coverage tooling SHALL track
`DECODE-COEFF-PARITY-TCQ-HANDOFF` as a partial loaded-but-unwired
decoder-support row linked to AV2 § 5.18.2, § 5.20.7.27, and the existing
coefficient frame-facts/base-q `useFsc` handoff rows.

#### Scenario: support matrix includes the row

- **WHEN** decoder support status is generated
- **THEN** it includes a partial row for `DECODE-COEFF-PARITY-TCQ-HANDOFF`
  describing the crate-private derivation of `parityHiding` and `useTcq` from
  parsed frame flags and block facts before base-q delegation
- **AND** it records that runtime `coeffs()` integration, full
  `compute_tx_type`, runtime block syntax traversal, segment-map derivation,
  dequantization, reconstruction, output, and reference refresh remain
  unsupported.

#### Scenario: conformance coverage maps the row

- **WHEN** decoder conformance coverage is checked
- **THEN** the coefficient syntax and decoder support coverage groups reference
  `DECODE-COEFF-PARITY-TCQ-HANDOFF`.
