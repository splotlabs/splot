## ADDED Requirements

### Requirement: lossless ordinary branch support row

The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-BRANCH-LOSSLESS-HANDOFF` as a distinct
loaded-but-unwired ordinary coefficient branch infrastructure row. The row SHALL
cite AV2 v1.0.0 §5.20.7.27, §5.20.7.29, §5.20.8.3, and §9.2; SHALL name
focused ordinary-branch tests as proof; and SHALL keep FSC/IDTX lossless cases,
inter/luma transform-state lookup, frame-state parsing, runtime `coeffs()`,
dequantization, reconstruction, output, reference refresh, and external decoder
invocation as residual work.

#### Scenario: Support matrix records lossless handoff only

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** a `coeff-ordinary-branch-lossless-handoff` row appears with Feature
  ID `DECODE-COEFF-ORDINARY-BRANCH-LOSSLESS-HANDOFF`
- **AND** broad coefficient-loop and runtime decode rows remain partial or
  unsupported until separately implemented
