## ADDED Requirements

### Requirement: Mode_To_Txfm support row

The decoder support model SHALL track `DECODE-MODE-TO-TXFM-SYMBOLIC-TABLE` as a
distinct generated-table infrastructure row. The row SHALL cite AV2 v1.0.0 §3
and §9.2, SHALL name generator and mirror-backed table tests as proof, and
SHALL keep AV2 §5.20.7.29 `compute_tx_type()`, `TxTypes` tile state, transform
set membership, runtime `coeffs()` wiring, dequantization, reconstruction,
output, reference refresh, and external decoder invocation as unsupported or
partial residual work.

#### Scenario: Support matrix records generated table only

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** a `mode-to-txfm-symbolic-table` row appears with Feature ID
  `DECODE-MODE-TO-TXFM-SYMBOLIC-TABLE`
- **AND** the row names generator and core table spot tests as proof
- **AND** broad coefficient-loop and runtime decode rows remain partial or
  unsupported until separately implemented
