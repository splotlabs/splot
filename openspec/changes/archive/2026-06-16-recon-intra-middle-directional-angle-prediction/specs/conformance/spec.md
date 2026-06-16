## ADDED Requirements

### Requirement: Middle Directional Angle Fuzz Evidence
The conformance metadata SHALL record that `recon_intra_prediction_bytes`
covers the source-backed middle directional-angle primitive. The fuzz coverage
SHALL remain a no-panic and typed-error proof for `splot-recon` only and SHALL
NOT claim AV2 bitstream decode, runtime output, AVM/dav2d agreement, or full
directional prediction conformance.

#### Scenario: Fuzz metadata includes middle angles
- **WHEN** `cargo xtask check-feature-status` and
  `cargo xtask check-decoder-support` validate status metadata
- **THEN** the fuzz row references
  `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION`
- **AND** the row continues to state that runtime decode, filesystem I/O,
  subprocesses, AVM, dav2d, and broad AV2 conformance are out of scope
