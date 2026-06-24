## ADDED Requirements

### Requirement: ac0ej3 luma transform-type residual support row

Decoder support tracking SHALL record the ac0ej3 luma transform-type residual
handoff as a partial row that advances the live stream beyond
`unsupported_dctonly_residual_luma_tx_type` by carrying resolved non-DCT luma
`PlaneTxType` into syntax-only LR tx-skip record derivation. The row SHALL keep
inverse transforms, residual addition, loop-restoration filtering, decoded
output, reference refresh, AVM/dav2d byte equality, and successful ac0ej3 decode
out of scope.

#### Scenario: Live frontier evidence is recorded

- **WHEN** the local `ac0ej3.ivf` probe advances after luma transform-type
  residual handoff
- **THEN** the decoder support matrix records the new unsupported frontier,
  feature ID, proof commands, and explicit non-goals
