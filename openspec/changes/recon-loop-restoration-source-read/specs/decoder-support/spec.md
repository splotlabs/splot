## ADDED Requirements

### Requirement: Loop-restoration source-read support row

The decoder support model SHALL track `RECON-LOOP-RESTORATION-SOURCE-READ` as a
distinct `splot-recon` row named `loop-restoration-source-read`. The row SHALL
mark only the AV2 section 7.20.2 source-sample immutable coded-storage frame
read as supported over caller-resolved luma bounds, sequence subsampling, and
caller-supplied `CurrFrame` / `CdefFrame` views, including validation that
caller-resolved chroma subsampling matches the source frame pixel format. It
SHALL keep full loop restoration, Wiener NS invocation, PC-Wiener classification,
GDF, BRU, runtime decode wiring, and ac0ej3 decode partial or unsupported until
separately proven.

#### Scenario: Matrix records narrow loop-restoration frame-read progress

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `loop-restoration-source-read` appears with Feature ID
  `RECON-LOOP-RESTORATION-SOURCE-READ`
- **AND** it cites AV2 section 7.20.2 and focused `splot-recon` tests
- **AND** it does not claim full loop restoration, runtime decode wiring, or
  successful ac0ej3 decode
