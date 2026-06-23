## ADDED Requirements

### Requirement: Wiener NS chroma primitive support row

The decoder support model SHALL track
`RECON-WIENERNS-CHROMA-FILTER-PRIMITIVE` as a distinct `splot-recon` row named
`wienerns-chroma-filter-primitive`. The row SHALL mark only the AV2 §7.20.3
chroma non-separable Wiener per-block/per-sample arithmetic as supported over
caller-resolved chroma source samples, luma source samples, luma downsampling
facts, and coefficients. It SHALL keep full loop restoration, §7.20.2 frame
reads, §7.20.4 PC-Wiener classification, restoration-unit syntax,
temporal/reference Wiener state, GDF/BRU, runtime decode wiring, and ac0ej3
decode partial or unsupported until separately proven.

#### Scenario: Matrix records narrow chroma loop-restoration progress

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `wienerns-chroma-filter-primitive` appears with Feature ID
  `RECON-WIENERNS-CHROMA-FILTER-PRIMITIVE`
- **AND** it cites AV2 §7.20.3 and focused `splot-recon` tests
- **AND** it does not claim full loop restoration, runtime decode wiring, GDF,
  BRU, or successful ac0ej3 decode
