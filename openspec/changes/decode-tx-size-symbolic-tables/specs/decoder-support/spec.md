## ADDED Requirements

### Requirement: TxSize symbolic tables support row
The decoder support model SHALL track `DECODE-TX-SIZE-SYMBOLIC-TABLES` as a
distinct infrastructure row named `tx-size-symbolic-tables`. The row SHALL mark
only generated AV2 section 9.2 TxSize enum-valued conversion table support as
partial decoder infrastructure and SHALL keep runtime coefficient-loop wiring,
`txSzCtx`, `compute_tx_type`, scan derivation, dequantization, reconstruction,
output, reference refresh, and broad `decode_block()` / `decode_tile()` support
honestly unsupported or partial.

#### Scenario: Matrix records generated-table support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `tx-size-symbolic-tables` appears with Feature ID
  `DECODE-TX-SIZE-SYMBOLIC-TABLES`
- **AND** it names focused generator/table tests plus `cargo xtask gen-tables --check`
- **AND** it does not claim runtime coefficient-loop integration or decoded
  output changes

#### Scenario: Generated support docs include the row
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** the new row is included in the generated status and coverage
  documents with status `partial`
