## ADDED Requirements

### Requirement: Middle Directional Angle Support Row
The decoder support model SHALL track
`RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION` as a distinct `splot-recon`
source-backed row named `intra-middle-directional-angle-prediction`. The row
SHALL mark only AV2 v1.0.0 7.13.2.8 non-IDIF pAngles `113`, `135`, and `157`
over caller-prepared logical edge ranges as supported, SHALL cite AV2 4.8,
7.13.2.1, 7.13.2.7, 7.13.2.8, and 9.2, and SHALL keep broad intra
reconstruction, edge preparation, IDIF, MRL, directional IBP, runtime decode,
transform/residual, loop-filter, and reference-refresh rows honestly partial
or unsupported.

#### Scenario: Matrix records narrow middle-angle support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `intra-middle-directional-angle-prediction` appears with Feature ID
  `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION`
- **AND** it names focused unit tests plus the extended
  `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim full edge preparation, IDIF, MRL, data-driven
  prediction, directional IBP, residuals, transforms, loop filters, reference
  refresh, film grain, AVM/dav2d evidence, or full decoder conformance

#### Scenario: Broad directional rows remain partial
- **WHEN** decoder support and conformance coverage status documents are
  regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, and other broad runtime
  decoder rows remain partial or unsupported until separately implemented with
  runtime evidence
