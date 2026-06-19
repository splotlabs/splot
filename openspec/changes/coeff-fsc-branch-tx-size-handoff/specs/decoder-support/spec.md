## ADDED Requirements

### Requirement: Track FSC Branch Tx-Size Handoff

The decoder support matrix and decoder conformance coverage tooling SHALL track
`DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF` as a partial loaded-but-unwired
decoder-support row linked to AV2 § 5.20.7.27, § 5.20.7.30, § 8.3.2, and the
generated § 9.2 transform-size tables.

#### Scenario: support matrix includes the row

- **WHEN** decoder support status is generated
- **THEN** it includes a partial row for
  `DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF` describing the crate-private FSC
  tx-size fact handoff and its remaining runtime/dequant/reconstruction gaps.

#### Scenario: conformance coverage maps the row

- **WHEN** decoder conformance coverage is checked
- **THEN** the coefficient syntax and decoder support coverage groups reference
  `DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF`.
