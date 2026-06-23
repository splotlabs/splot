## ADDED Requirements

### Requirement: chroma-inter TxTypes ordinary branch support row

The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-BRANCH-CHROMA-INTER-TXTYPES-HANDOFF` as a distinct
loaded-but-unwired ordinary coefficient branch infrastructure row. The row SHALL
record AV2 section 5.20.7.29 chroma-inter `TxTypes[y4][x4]` transform-type
derivation, focused ordinary branch tests, and the remaining runtime decoder
gaps.

#### Scenario: Chroma-inter TxTypes handoff appears in decoder support

- **WHEN** decoder support status is generated
- **THEN** it SHALL include a partial row with Feature ID
  `DECODE-COEFF-ORDINARY-BRANCH-CHROMA-INTER-TXTYPES-HANDOFF`
- **AND** the row SHALL state that runtime `coeffs()` wiring, frame-state
  `TxTypes`/`MiRow`/`MiCol`/subsampling derivation, FSC/IDTX lossless handling,
  reconstruction, output, and AVM/dav2d proof remain unsupported
