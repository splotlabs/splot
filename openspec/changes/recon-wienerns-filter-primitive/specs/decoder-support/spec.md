## ADDED Requirements

### Requirement: Wiener NS luma primitive support row

The decoder support model SHALL track `RECON-WIENERNS-FILTER-PRIMITIVE` as a
distinct `splot-recon` row named `wienerns-filter-primitive`. The row SHALL mark
only the AV2 §7.20.3 luma non-separable Wiener per-block/per-sample arithmetic as
supported over caller-resolved source samples, subclasses, and coefficients. It
SHALL keep full loop restoration, §7.20.2 source-sample clipping/stripe handling,
§7.20.4 PC-Wiener classification, chroma Wiener NS filtering, restoration-unit
syntax, temporal/reference Wiener state, runtime decode wiring, and local decoder mission decode
partial or unsupported until separately proven.

#### Scenario: Matrix records narrow loop-restoration progress

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `wienerns-filter-primitive` appears with Feature ID
  `RECON-WIENERNS-FILTER-PRIMITIVE`
- **AND** it cites AV2 §7.20.3 and focused `splot-recon` tests
- **AND** it does not claim full loop restoration, runtime decode wiring, chroma
  Wiener NS support, or successful local decoder mission decode
