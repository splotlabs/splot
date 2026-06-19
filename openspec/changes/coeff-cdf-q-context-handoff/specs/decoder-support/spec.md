## ADDED Requirements

### Requirement: Track Coefficient CDF Q-Context Handoff

The decoder support matrix and decoder conformance coverage tooling SHALL track
`DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF` as a partial loaded-but-unwired
decoder-support row linked to AV2 § 3, § 5.20.7.27, § 6.17.2, and the existing
shared-facts `useFsc` handoff row.

#### Scenario: support matrix includes the row

- **WHEN** decoder support status is generated
- **THEN** it includes a partial row for
  `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF` describing the crate-private wrapper that
  derives `coeff_cdf_q_ctx` from frame `base_q_idx` before delegating to the
  shared-facts `useFsc` handoff
- **AND** it records that runtime `coeffs()` integration, full CDF lifecycle
  wiring, full `compute_tx_type`, runtime fact derivation, dequantization,
  reconstruction, output, and reference refresh remain unsupported.

#### Scenario: conformance coverage maps the row

- **WHEN** decoder conformance coverage is checked
- **THEN** the coefficient syntax and decoder support coverage groups reference
  `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF`.
