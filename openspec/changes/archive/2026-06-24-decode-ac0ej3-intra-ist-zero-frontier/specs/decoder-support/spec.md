## ADDED Requirements

### Requirement: ac0ej3 intra IST zero support row

The decoder support matrix SHALL include
`DECODE-AC0EJ3-INTRA-IST-ZERO-FRONTIER` as a distinct partial ac0ej3 row named
`ac0ej3-intra-ist-zero-frontier`. The row SHALL record that the decoder consumes
the covered `sec_tx_type == 0` intra IST syntax and remains fail-closed for
active secondary transforms and successful ac0ej3 decode output.

#### Scenario: Support matrix records intra IST zero frontier

- **WHEN** decoder support status is generated
- **THEN** `ac0ej3-intra-ist-zero-frontier` appears with Feature ID
  `DECODE-AC0EJ3-INTRA-IST-ZERO-FRONTIER`
- **AND** it lists focused CDF/residual tests plus the local ac0ej3 runtime
  probe
- **AND** it does not claim successful ac0ej3 decode, raw/Y4M output, reference
  refresh, or AVM/dav2d byte equality
