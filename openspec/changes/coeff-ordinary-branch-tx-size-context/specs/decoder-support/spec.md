## ADDED Requirements

### Requirement: Ordinary branch transform-size-context support row
The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-CONTEXT` as a distinct partial decoder row
for the loaded-but-unwired ordinary coefficient branch. The row SHALL record
that `txSzCtx` is derived from generated AV2 section 9.2 tables while keeping
runtime coefficient-loop integration, `compute_tx_type`, scan derivation,
dequantization, reconstruction, output, reference refresh, and broad
`decode_block()` / `decode_tile()` support honestly unsupported or partial.

#### Scenario: Matrix records transform-size-context handoff

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** the transform-size-context ordinary branch row appears with Feature ID
  `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-CONTEXT`
- **AND** it names focused ordinary branch tests plus the feature/status checks
- **AND** it does not claim runtime coefficient-loop integration or decoded
  output changes

#### Scenario: Generated support docs include the row

- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** the new row is included with status `partial`
