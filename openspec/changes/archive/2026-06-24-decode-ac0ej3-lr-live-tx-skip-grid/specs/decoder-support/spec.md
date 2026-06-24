## ADDED Requirements

### Requirement: ac0ej3 LR live tx-skip grid support row

The decoder support model SHALL track
`DECODE-AC0EJ3-LR-LIVE-TX-SKIP-GRID` as a distinct ac0ej3 row named
`ac0ej3-lr-live-tx-skip-grid`. The row SHALL record that the decoder can
populate the live allocated `LrTxSkip` shell from a complete retained
`WienerNsLrTxSkipGrid`, and SHALL keep live decoded samples, tile-derived
transform-record handoff, `FilterClass` retention, `SubclassLookup`, loop-
restoration filtering/output, reference refresh, AVM/dav2d byte equality, and
successful ac0ej3 decode unsupported until separately proven.

#### Scenario: Support matrix lists live tx-skip grid population

- **WHEN** `cargo xtask check-decoder-support` validates decoder support rows
- **THEN** `ac0ej3-lr-live-tx-skip-grid` appears with Feature ID
  `DECODE-AC0EJ3-LR-LIVE-TX-SKIP-GRID`
- **AND** it cites focused live-storage tests
- **AND** it remains `partial`
