## ADDED Requirements

### Requirement: Decoder support matrix tracks intra prediction fuzz coverage

The decoder support matrix SHALL include a row named
`recon-intra-prediction-fuzz`, tracked by Feature ID
`CONF-RECON-INTRA-PREDICTION-FUZZ`, covering no-panic fuzz coverage for
source-backed `splot-recon` intra prediction and current-frame workspace
primitives over bounded structured inputs.

#### Scenario: intra prediction fuzz row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `recon-intra-prediction-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/recon_intra_prediction_bytes.rs` as
  evidence
- **AND** it records bounded structured prediction/workspace generation and
  fuzz target enumeration commands
- **AND** it does not mark broad runtime decode, full §7.13 intra
  reconstruction, directional prediction, data driven intra prediction, IBP,
  filter intra, CfL/CCTX, palette, residual, transform, quantization,
  loop-filter, AVM/dav2d differential testing, filesystem publication, or
  output scheduling as supported
