## ADDED Requirements

### Requirement: local decoder mission LR live storage allocation support row

The decoder support model SHALL track
`DECODE-LR-LIVE-STORAGE-ALLOCATION` as a distinct local decoder mission row named
`lr-live-storage-allocation`. The row SHALL record that the live local decoder mission
path allocates unpopulated active-bit-depth loop-restoration frame-buffer
shells and an unpopulated `LrTxSkip` grid after storage-footprint planning, and
SHALL keep decoded sample population, real `LrTxSkip` values, `FilterClass`
grid retention, `SubclassLookup`, loop-restoration filtering, 10-bit output,
reference refresh, and successful local decoder mission decode unsupported until separately
proven.

#### Scenario: Matrix records live storage allocation frontier
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `lr-live-storage-allocation` appears with Feature ID
  `DECODE-LR-LIVE-STORAGE-ALLOCATION`
- **AND** it cites AV2 §6.4.1, §6.17.4.1, §7.20.2, §7.20.3, and §7.20.4
- **AND** it records `decode/unsupported-feature` with the live-storage
  allocation diagnostic
- **AND** it does not claim loop-restoration filtering, output, reference
  refresh, AVM/dav2d byte equality, or successful local decoder mission decode
