## ADDED Requirements

### Requirement: ac0ej3 LR live transform-record handoff support row

The decoder support model SHALL track
`DECODE-AC0EJ3-LR-LIVE-TRANSFORM-RECORD-HANDOFF` as a distinct partial row named
`ac0ej3-lr-live-transform-record-handoff`. The row SHALL record that the decoder
can hand fixed-largest parsed luma transform records into live LR `LrTxSkip`
storage, and that the local ac0ej3 stream is now blocked on selectable
transform-record parsing before live `LrTxSkip` values can be populated from the
key tile.

#### Scenario: Support matrix lists transform-record handoff frontier

- **WHEN** `cargo xtask check-decoder-support` validates decoder support rows
- **THEN** `ac0ej3-lr-live-transform-record-handoff` appears with Feature ID
  `DECODE-AC0EJ3-LR-LIVE-TRANSFORM-RECORD-HANDOFF`
- **AND** it remains `partial`
- **AND** it does not claim selectable transform partition parsing,
  `FilterClass`, `SubclassLookup`, loop-restoration filtering/output, reference
  refresh, AVM/dav2d byte equality, or successful ac0ej3 decode
