## ADDED Requirements

### Requirement: Cardinal Directional Prediction Support Row
The decoder support model SHALL track
`RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION` as a distinct `splot-recon`
source-backed row named `intra-cardinal-directional-prediction`. The row SHALL
mark only H/V pAngle 90/180 scalar prediction and workspace handoff as
supported, SHALL cite AV2 v1.0.0 §7.13.2.1, §7.13.2.7, §7.13.2.8, and §9.2,
and SHALL keep broad intra reconstruction, broad directional prediction,
runtime decode, transform/residual, loop-filter, and reference-refresh rows
honestly partial or unsupported.

#### Scenario: Matrix records narrow support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `intra-cardinal-directional-prediction` appears with Feature ID
  `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION`
- **AND** it names focused unit/workspace tests plus the extended
  `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim general directional angles, IDIF, MRL, IBP,
  wide-angle mapping, CfL/CCTX/MHCCP, palette, residuals, transforms, loop
  filters, reference refresh, film grain, AVM/dav2d evidence, or full decoder
  conformance

#### Scenario: Broad rows remain partial
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, and other broad runtime
  decoder rows remain partial or unsupported until separately implemented with
  runtime evidence
