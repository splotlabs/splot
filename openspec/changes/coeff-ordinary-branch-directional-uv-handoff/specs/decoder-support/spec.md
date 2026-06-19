## ADDED Requirements

### Requirement: directional UV ordinary branch support row

The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF` as a distinct
loaded-but-unwired ordinary coefficient branch infrastructure row. The row SHALL
record AV2 section 5.20.7.29 directional `UVMode` transform-type derivation,
generated table use, focused ordinary branch tests, and the remaining runtime
decoder gaps.

#### Scenario: Directional UV handoff appears in decoder support

- **WHEN** decoder support status is generated
- **THEN** it SHALL include a partial row with Feature ID
  `DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF`
- **AND** the row SHALL state that runtime `coeffs()` wiring, luma/inter
  `TxTypes` lookup, FSC/IDTX lossless handling, reconstruction, output, and
  AVM/dav2d proof remain unsupported
