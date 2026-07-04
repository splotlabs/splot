## ADDED Requirements

### Requirement: local decoder mission selectable transform-record support row

The decoder support model SHALL track
`DECODE-SELECTABLE-TRANSFORM-RECORDS` as a distinct partial row named
`selectable-transform-records`. The row SHALL record that the decoder can
parse supported `TX_MODE_SELECT` luma transform-size/partition records for the
local decoder mission LR path, feed the resulting transform facts into live `LrTxSkip` storage,
and then stop before decoded sample population and LR filtering.

#### Scenario: Support matrix lists selectable transform-record frontier

- **WHEN** `cargo xtask check-decoder-support` validates decoder support rows
- **THEN** `selectable-transform-records` appears with Feature ID
  `DECODE-SELECTABLE-TRANSFORM-RECORDS`
- **AND** it remains `partial`
- **AND** it does not claim decoded frame samples, `FilterClass`,
  `SubclassLookup`, loop-restoration filtering/output, reference refresh,
  AVM/dav2d byte equality, or successful local decoder mission decode

## MODIFIED Requirements

### Requirement: local decoder mission LR live transform-record handoff support row

The decoder support model SHALL track
`DECODE-LR-LIVE-TRANSFORM-RECORD-HANDOFF` as a distinct partial row named
`lr-live-transform-record-handoff`. The row SHALL record that the decoder
can hand fixed-largest parsed luma transform records into live LR `LrTxSkip`
storage, and that the local decoder mission stream's selectable-transform record parsing
is now tracked by `DECODE-SELECTABLE-TRANSFORM-RECORDS`.

#### Scenario: Support matrix lists transform-record handoff frontier

- **WHEN** `cargo xtask check-decoder-support` validates decoder support rows
- **THEN** `lr-live-transform-record-handoff` appears with Feature ID
  `DECODE-LR-LIVE-TRANSFORM-RECORD-HANDOFF`
- **AND** it remains `partial`
- **AND** it does not claim decoded frame samples, `FilterClass`,
  `SubclassLookup`, loop-restoration filtering/output, reference refresh,
  AVM/dav2d byte equality, or successful local decoder mission decode
