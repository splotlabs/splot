## ADDED Requirements

### Requirement: Ordinary branch adjusted transform-size support row
The decoder support model SHALL track
`DECODE-COEFF-ORDINARY-BRANCH-ADJUSTED-TX-SIZE` as a distinct partial decoder
row for the loaded-but-unwired ordinary coefficient branch. The row SHALL record
that adjusted transform-size dimensions are derived for ordinary base contexts
from generated AV2 section 9.2 tables while keeping runtime coefficient-loop
integration, `txSzCtx`, `compute_tx_type`, scan derivation, dequantization,
reconstruction, output, reference refresh, and broad `decode_block()` /
`decode_tile()` support honestly unsupported or partial.

#### Scenario: Matrix records adjusted-size handoff
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** the adjusted-size ordinary branch row appears with Feature ID
  `DECODE-COEFF-ORDINARY-BRANCH-ADJUSTED-TX-SIZE`
- **AND** it names focused ordinary branch tests plus the feature/status checks
- **AND** it does not claim runtime coefficient-loop integration or decoded
  output changes

#### Scenario: Generated support docs include the row
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** the new row is included with status `partial`
