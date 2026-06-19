## ADDED Requirements

### Requirement: Track useFsc Shared-Facts Handoff

The decoder support matrix and decoder conformance coverage tooling SHALL track
`DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF` as a partial loaded-but-unwired
decoder-support row linked to AV2 section 5.20.7.27 and the existing `useFsc`
branch and condition handoff rows.

#### Scenario: support matrix includes the row

- **WHEN** decoder support status is generated
- **THEN** it includes a partial row for
  `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF` describing the crate-private
  wrapper that derives `useFsc` from one shared nonzero fact packet and lazily
  constructs only the selected lower branch input
- **AND** it records that runtime `coeffs()` integration, full
  `compute_tx_type`, runtime fact derivation, dequantization, reconstruction,
  output, and reference refresh remain unsupported.

#### Scenario: conformance coverage maps the row

- **WHEN** decoder conformance coverage is checked
- **THEN** the coefficient syntax and decoder support coverage groups reference
  `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF`.
