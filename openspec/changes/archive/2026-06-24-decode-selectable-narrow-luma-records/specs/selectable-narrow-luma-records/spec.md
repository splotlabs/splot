## ADDED Requirements

### Requirement: local decoder mission Selectable Narrow Luma Records

The decoder SHALL track `DECODE-SELECTABLE-NARROW-LUMA-RECORDS` as a
partial runtime prerequisite for the local decoder mission Wiener NS LR path. When the local
stream's selectable transform-record handoff reaches a luma-only SDP leaf with
valid nonzero luma dimensions, including the observed `BLOCK_4X32` case, the
runtime SHALL consume AV2 §5.20.6.1/§5.20.6.3 transform-size syntax and
§5.20.7.27 luma coefficient syntax needed to derive `LrTxSkip`, without
requiring chroma syntax for that leaf. The runtime SHALL remain fail-closed
before decoded sample population or output.

#### Scenario: Luma-only BLOCK_4X32 advances the live frontier

- **WHEN** the local decoder mission stream reaches the luma-only `BLOCK_4X32`
  selectable transform-record leaf at the current live gate
- **THEN** the runtime consumes the luma mode, transform-size, and luma
  coefficient syntax needed for its `LrTxSkip` transform records
- **AND** it no longer emits
  `unsupported_wienerns_lr_selectable_transform_records_block_shape` for that
  leaf
- **AND** it stops at the next structured unsupported frontier before output

#### Scenario: Chroma claims remain excluded

- **WHEN** luma-only narrow selectable transform records have been consumed
- **THEN** the decoder SHALL NOT claim narrow chroma prediction, CfL prediction,
  decoded chroma samples, decoded `CurrFrame` or `CdefFrame` samples,
  `FilterClass` retention, loop-restoration filtering/output, reference
  refresh, AVM/dav2d byte equality, or successful local decoder mission decode
