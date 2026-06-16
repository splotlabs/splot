## ADDED Requirements

### Requirement: IBP DC Prediction Support Row
The decoder support model SHALL track `RECON-INTRA-IBP-DC-PREDICTION` as a
distinct `splot-recon` source-backed row named `intra-ibp-dc-prediction`. The
row SHALL mark only AV2 §7.13.2.12 prepared-edge scalar prediction and
workspace handoff as supported, SHALL cite AV2 v1.0.0 §3, §4.8, §7.13.2.1,
§7.13.2.10, and §7.13.2.12, and SHALL keep broad intra reconstruction, full
`predict_intra()` dispatch, general directional IBP, runtime decode,
transform/residual, loop-filter, and reference-refresh rows honestly partial or
unsupported.

#### Scenario: Matrix records narrow support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `intra-ibp-dc-prediction` appears with Feature ID
  `RECON-INTRA-IBP-DC-PREDICTION`
- **AND** it names focused unit/workspace tests plus the extended
  `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim full edge preparation, full dispatcher support,
  directional IBP, data-driven prediction, CfL/CCTX/MHCCP, residuals,
  transforms, loop filters, reference refresh, film grain, AVM/dav2d evidence,
  or full decoder conformance

#### Scenario: Broad rows remain partial
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, and other broad runtime
  decoder rows remain partial or unsupported until separately implemented with
  runtime evidence
