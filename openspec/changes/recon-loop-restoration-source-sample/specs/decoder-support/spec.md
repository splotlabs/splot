## ADDED Requirements

### Requirement: Loop-restoration source-sample support row

The decoder support model SHALL track `RECON-LOOP-RESTORATION-SOURCE-SAMPLE` as
a distinct `splot-recon` row named `loop-restoration-source-sample`. The row
SHALL mark only the AV2 section 7.20.2 source-sample coordinate clipping and
`CurrFrame` / `CdefFrame` source selection as supported over caller-resolved
luma bounds and sequence subsampling. It SHALL keep full loop restoration, frame
storage reads, Wiener NS filtering, PC-Wiener classification, GDF, BRU, runtime
decode wiring, and local decoder mission decode partial or unsupported until separately proven.

#### Scenario: Matrix records narrow loop-restoration source progress

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `loop-restoration-source-sample` appears with Feature ID
  `RECON-LOOP-RESTORATION-SOURCE-SAMPLE`
- **AND** it cites AV2 section 7.20.2 and focused `splot-recon` tests
- **AND** it does not claim full loop restoration, frame reads, runtime decode
  wiring, or successful local decoder mission decode
