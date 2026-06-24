## ADDED Requirements

### Requirement: ac0ej3 LR live storage allocation support row

The decoder support model SHALL track
`DECODE-AC0EJ3-LR-LIVE-STORAGE-ALLOCATION` as a distinct ac0ej3 row named
`ac0ej3-lr-live-storage-allocation`. The row SHALL record that the live ac0ej3
path allocates unpopulated active-bit-depth loop-restoration frame-buffer
shells and an unpopulated `LrTxSkip` grid after storage-footprint planning, and
SHALL keep decoded sample population, real `LrTxSkip` values, `FilterClass`
grid retention, `SubclassLookup`, loop-restoration filtering, 10-bit output,
reference refresh, and successful ac0ej3 decode unsupported until separately
proven.

#### Scenario: Matrix records live storage allocation frontier
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `ac0ej3-lr-live-storage-allocation` appears with Feature ID
  `DECODE-AC0EJ3-LR-LIVE-STORAGE-ALLOCATION`
- **AND** it cites AV2 §6.4.1, §6.17.4.1, §7.20.2, §7.20.3, and §7.20.4
- **AND** it records `decode/unsupported-feature` with the live-storage
  allocation diagnostic
- **AND** it does not claim loop-restoration filtering, output, reference
  refresh, AVM/dav2d byte equality, or successful ac0ej3 decode
