## ADDED Requirements

### Requirement: Track useFsc Branch Handoff

The decoder support matrix and decoder conformance coverage tooling SHALL track
`DECODE-COEFF-USE-FSC-BRANCH-HANDOFF` as a partial loaded-but-unwired
decoder-support row linked to AV2 § 5.20.7.27 and the existing ordinary/FSC
coefficient branch handoff rows.

#### Scenario: support matrix includes the row

- **WHEN** decoder support status is generated
- **THEN** it includes a partial row for
  `DECODE-COEFF-USE-FSC-BRANCH-HANDOFF` describing the crate-private selector
  that routes all-zero to ordinary, nonzero `useFsc == false` to ordinary, and
  nonzero `useFsc == true` to FSC
- **AND** it records that runtime `useFsc` derivation, runtime `coeffs()`,
  dequantization, reconstruction, output, and reference refresh remain
  unsupported.

#### Scenario: conformance coverage maps the row

- **WHEN** decoder conformance coverage is checked
- **THEN** the coefficient syntax and decoder support coverage groups reference
  `DECODE-COEFF-USE-FSC-BRANCH-HANDOFF`.
