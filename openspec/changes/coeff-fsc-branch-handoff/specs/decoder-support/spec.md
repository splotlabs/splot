## ADDED Requirements

### Requirement: Coefficient FSC branch handoff support status

The decoder support model SHALL track `DECODE-COEFF-FSC-BRANCH-HANDOFF` as a
distinct crate-private partial decoder boundary named `coeff-fsc-branch-handoff`.
The row SHALL mark only loaded-but-unwired FSC/IDTX nonzero coefficient branch
composition as partial support, and SHALL keep runtime `coeffs()`,
dequantization, reconstruction, output, reference refresh, public APIs, and
AVM/dav2d evidence unsupported until separately implemented.

#### Scenario: Matrix records the FSC branch handoff boundary

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-fsc-branch-handoff` appears with Feature ID
  `DECODE-COEFF-FSC-BRANCH-HANDOFF`
- **AND** it cites AV2 §5.20.7.27 for the `useFsc` nonzero branch and FSC scan
  loops plus §5.20.7.28 for the quant syntax reached by that branch
- **AND** it names focused tests for explicit-pipeline equivalence,
  all-zero-routing rejection, invalid scan rejection, and chroma-routing
  rejection
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
