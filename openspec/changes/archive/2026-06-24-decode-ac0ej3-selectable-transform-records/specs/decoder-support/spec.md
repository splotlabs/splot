## ADDED Requirements

### Requirement: ac0ej3 selectable transform-record support row

The decoder support model SHALL track
`DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS` as a distinct partial row named
`ac0ej3-selectable-transform-records`. The row SHALL record that the decoder can
parse supported `TX_MODE_SELECT` luma transform-size/partition records for the
ac0ej3 LR path, feed the resulting transform facts into live `LrTxSkip` storage,
and then stop before decoded sample population and LR filtering.

#### Scenario: Support matrix lists selectable transform-record frontier

- **WHEN** `cargo xtask check-decoder-support` validates decoder support rows
- **THEN** `ac0ej3-selectable-transform-records` appears with Feature ID
  `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`
- **AND** it remains `partial`
- **AND** it does not claim decoded frame samples, `FilterClass`,
  `SubclassLookup`, loop-restoration filtering/output, reference refresh,
  AVM/dav2d byte equality, or successful ac0ej3 decode

## MODIFIED Requirements

### Requirement: ac0ej3 LR live transform-record handoff support row

The decoder support model SHALL track
`DECODE-AC0EJ3-LR-LIVE-TRANSFORM-RECORD-HANDOFF` as a distinct partial row named
`ac0ej3-lr-live-transform-record-handoff`. The row SHALL record that the decoder
can hand fixed-largest parsed luma transform records into live LR `LrTxSkip`
storage, and that the local ac0ej3 stream's selectable-transform record parsing
is now tracked by `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`.

#### Scenario: Support matrix lists transform-record handoff frontier

- **WHEN** `cargo xtask check-decoder-support` validates decoder support rows
- **THEN** `ac0ej3-lr-live-transform-record-handoff` appears with Feature ID
  `DECODE-AC0EJ3-LR-LIVE-TRANSFORM-RECORD-HANDOFF`
- **AND** it remains `partial`
- **AND** it does not claim decoded frame samples, `FilterClass`,
  `SubclassLookup`, loop-restoration filtering/output, reference refresh,
  AVM/dav2d byte equality, or successful ac0ej3 decode
