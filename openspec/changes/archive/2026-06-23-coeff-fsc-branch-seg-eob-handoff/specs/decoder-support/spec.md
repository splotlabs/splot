## ADDED Requirements

### Requirement: Coefficient FSC segment handoff support status

The decoder support model SHALL track
`DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF` as a distinct crate-private partial
decoder boundary named `coeff-fsc-branch-seg-eob-handoff`.

#### Scenario: Matrix records the FSC segment handoff boundary

- **WHEN** decoder support status is generated
- **THEN** `coeff-fsc-branch-seg-eob-handoff` appears with Feature ID
  `DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF`
- **AND** the row remains partial because runtime `coeffs()` and scan derivation
  are not wired

#### Scenario: Conformance coverage links the feature

- **WHEN** decoder conformance coverage is checked
- **THEN** the FSC coefficient coverage group references both support row
  `coeff-fsc-branch-seg-eob-handoff` and Feature ID
  `DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF`
