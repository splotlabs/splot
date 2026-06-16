## ADDED Requirements

### Requirement: One-Sided Directional Angle Support Row
The decoder support model SHALL track
`RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION` as a distinct
`splot-recon` source-backed row named
`intra-one-sided-directional-angle-prediction`. The row SHALL mark only AV2
§7.13.2.8 prepared-edge non-IDIF one-sided pAngles `45`, `67`, and `203` as
supported, SHALL cite AV2 v1.0.0 §7.13.2.1, §7.13.2.7, §7.13.2.8, and §9.2,
and SHALL keep broad intra reconstruction, full directional dispatch, middle
angles, luma IDIF, MRL, directional IBP, runtime decode, transform/residual,
loop-filter, and reference-refresh rows honestly partial or unsupported.

#### Scenario: Matrix records narrow directional-angle support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `intra-one-sided-directional-angle-prediction` appears with Feature
  ID `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`
- **AND** it names focused direct primitive tests plus the extended
  `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim full edge preparation, pAngles outside `45`, `67`,
  and `203`, luma IDIF, MRL, directional IBP, workspace synthesis, runtime
  decode, residuals, transforms, loop filters, reference refresh, film grain,
  AVM/dav2d evidence, or full decoder conformance

#### Scenario: Broad rows remain partial
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, and other broad runtime
  decoder rows remain partial or unsupported until separately implemented with
  runtime evidence
