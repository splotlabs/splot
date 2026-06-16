## ADDED Requirements

### Requirement: DC Subsampled Prediction Support Row
The decoder support model SHALL track
`RECON-INTRA-DC-SUBSAMPLED-PREDICTION` as a distinct `splot-recon`
source-backed row named `intra-dc-subsampled-prediction`. The row SHALL mark
only AV2 §7.13.2.11 prepared-edge scalar prediction and workspace handoff as
supported, SHALL cite AV2 v1.0.0 §7.13.2.1, §7.13.2.11, and §7.13.3.22, and
SHALL keep broad intra reconstruction, full `predict_intra()` dispatch, CfL,
runtime decode, transform/residual, loop-filter, and reference-refresh rows
honestly partial or unsupported.

#### Scenario: Matrix records narrow support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `intra-dc-subsampled-prediction` appears with Feature ID
  `RECON-INTRA-DC-SUBSAMPLED-PREDICTION`
- **AND** it names focused unit/workspace tests plus the extended
  `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim full CfL, data-driven prediction, IBP, general
  directional prediction, residuals, transforms, loop filters, reference
  refresh, film grain, AVM/dav2d evidence, or full decoder conformance

#### Scenario: Broad rows remain partial
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, and other broad runtime
  decoder rows remain partial or unsupported until separately implemented with
  runtime evidence
