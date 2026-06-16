## ADDED Requirements

### Requirement: Workspace Directional Angle Support Row
The decoder support model SHALL track `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION` as a distinct `splot-recon` source-backed row named `workspace-directional-angle-prediction`. The row SHALL mark only current-frame chroma/no-IDIF workspace handoff for fully available in-storage one-sided pAngles `45`, `67`, and `203`, and middle pAngles `113`, `135`, and `157` as supported; SHALL record that `PlaneId::Y` is rejected until luma IDIF is implemented; SHALL cite AV2 v1.0.0 §4.8, §7.13.2.1, §7.13.2.7, §7.13.2.8, and §9.2; and SHALL keep broad intra reconstruction, fallback edge preparation, runtime decode, transform/residual, loop-filter, and reference-refresh rows honestly partial or unsupported.

#### Scenario: Matrix records workspace directional-angle support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support matrix
- **THEN** `workspace-directional-angle-prediction` appears with Feature ID `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION`
- **AND** it names focused workspace tests plus the extended `recon_intra_prediction_bytes` fuzz target
- **AND** it does not claim full edge preparation, pAngles outside the modeled one-sided and middle subsets, luma IDIF, MRL, directional IBP, runtime decode, residuals, transforms, loop filters, reference refresh, film grain, AVM/dav2d evidence, or full decoder conformance

#### Scenario: Broad workspace and decoder rows remain partial
- **WHEN** decoder support and conformance coverage status documents are regenerated
- **THEN** `intra-reconstruction`, `prediction-process`, runtime decode, and other broad decoder rows remain partial or unsupported until separately implemented with runtime evidence
