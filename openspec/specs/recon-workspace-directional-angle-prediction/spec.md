# recon-workspace-directional-angle-prediction Specification

## Purpose
TBD - created by archiving change recon-workspace-directional-angle-prediction. Update Purpose after archive.
## Requirements
### Requirement: Workspace one-sided directional-angle prediction
`splot-recon` SHALL provide scheduler-free current-frame chroma/no-IDIF workspace helpers for the source-backed one-sided AV2 v1.0.0 §7.13.2.8 directional-angle pAngles `45`, `67`, and `203`, tracked by Feature ID `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION`. The helpers SHALL validate the target plane and rectangle, reject `PlaneId::Y` with a typed luma-IDIF-unsupported error until luma IDIF is implemented, gather only fully available in-storage prepared `AboveRow[0..w+h)` or `LeftCol[0..w+h)` samples for chroma planes, call the existing one-sided directional-angle primitive, and write the predicted rectangle into workspace storage.

#### Scenario: Workspace predicts one-sided above-edge angles
- **WHEN** a caller predicts pAngle `45` or `67` for a workspace rectangle whose full `AboveRow[0..w+h)` range is inside the plane storage
- **THEN** the workspace gathers the above samples from the row immediately above the target
- **AND** it writes the same samples that the direct one-sided directional-angle primitive would write for those prepared edges

#### Scenario: Workspace predicts one-sided left-edge angle
- **WHEN** a caller predicts pAngle `203` for a workspace rectangle whose full `LeftCol[0..w+h)` range is inside the plane storage
- **THEN** the workspace gathers the left samples from the column immediately left of the target
- **AND** it writes the same samples that the direct one-sided directional-angle primitive would write for that prepared edge

#### Scenario: Workspace one-sided errors are typed and non-mutating
- **WHEN** the target rectangle is outside storage, the target plane is `PlaneId::Y`, the required prepared edge is outside storage, an edge sample exceeds the active bit depth, the sample type cannot represent the active bit depth, or scratch allocation fails
- **THEN** the helper returns a structured `ReconError`
- **AND** it does not panic, silently synthesize AV2 fallback samples, emit validator diagnostics, or partially mutate the target rectangle

### Requirement: Workspace middle directional-angle prediction
`splot-recon` SHALL provide scheduler-free current-frame chroma/no-IDIF workspace helpers for the source-backed middle AV2 v1.0.0 §7.13.2.8 directional-angle pAngles `113`, `135`, and `157`, tracked by Feature ID `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION`. The helpers SHALL validate the target plane and rectangle, reject `PlaneId::Y` with a typed luma-IDIF-unsupported error until luma IDIF is implemented, gather only fully available in-storage logical `AboveRow[-1..w)` and `LeftCol[-1..h)` samples with slice index zero representing the logical `-1` sample for chroma planes, call the existing middle directional-angle primitive, and write the predicted rectangle into workspace storage.

#### Scenario: Workspace predicts middle angles
- **WHEN** a caller predicts pAngle `113`, `135`, or `157` for a workspace rectangle whose logical above and left prepared-edge ranges are fully inside plane storage
- **THEN** the workspace gathers `AboveRow[-1..w)` from the row immediately above the target and `LeftCol[-1..h)` from the column immediately left of the target
- **AND** it writes the same samples that the direct middle directional-angle primitive would write for those prepared edges

#### Scenario: Workspace middle errors are typed and non-mutating
- **WHEN** the target rectangle is outside storage, the target plane is `PlaneId::Y`, either logical prepared-edge range is outside storage, an edge sample exceeds the active bit depth, the sample type cannot represent the active bit depth, or scratch allocation fails
- **THEN** the helper returns a structured `ReconError`
- **AND** it does not panic, silently synthesize AV2 fallback samples, emit validator diagnostics, or partially mutate the target rectangle

### Requirement: Workspace directional-angle exclusions
The workspace directional-angle helpers SHALL NOT implement AV2 §7.13.2.1 fallback edge preparation, AV2 §7.13.2.7 full directional mode dispatch, MRL, luma IDIF, angle-delta derivation, wide-angle mapping, directional IBP, data-driven prediction, CfL/CCTX/MHCCP, palette, residual, transform, quantization, loop filtering, reference refresh, runtime `splot decode`, reference-decoder invocation, new dependencies, or scheduler state.

#### Scenario: Broad directional behavior remains out of scope
- **WHEN** a caller needs unavailable-edge fallback synthesis, mode dispatch, pAngles outside the modeled one-sided and middle subsets, runtime decode integration, or AVM/dav2d agreement
- **THEN** these workspace helpers do not claim support for that behavior
- **AND** decoder support and conformance metadata keep broad decoder rows partial or unsupported until separately implemented and proven
