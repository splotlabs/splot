# recon-intra-directional-angle-prediction Specification

## Purpose
TBD - created by archiving change recon-intra-directional-angle-prediction. Update Purpose after archive.
## Requirements
### Requirement: One-Sided Directional Angle Primitive
`splot-recon` SHALL provide a scheduler-free scalar primitive for the
source-backed one-sided chroma directional-angle cases from AV2 v1.0.0
§7.13.2.8, tracked by Feature ID
`RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`. The primitive SHALL accept
an active bit depth, rectangular block size, explicit supported pAngle, prepared
edge samples, caller-owned strided output storage, and SHALL validate sample
type, pAngle support, edge length, edge sample range, output stride, output
length, and checked arithmetic before reporting success. Supported pAngles SHALL
be limited to `45`, `67`, and `203`; the primitive SHALL reject pAngles `90`,
`113`, `135`, `157`, `180`, `270`, and any other unsupported value with a typed
`splot-recon` error.

#### Scenario: Above-row pAngle predicts with bilinear interpolation
- **WHEN** a caller predicts an 8-bit or 10-bit block with pAngle `45` or `67`
  and a prepared above edge of length `w + h`
- **THEN** each output sample is computed from AV2 §7.13.2.8 non-IDIF
  interpolation over `AboveRow` using the §9.2 derivative entry for that pAngle
- **AND** samples whose computed base reaches the prepared-edge limit use the
  edge-end fallback from §7.13.2.8

#### Scenario: Left-column pAngle predicts with bilinear interpolation
- **WHEN** a caller predicts an 8-bit or 10-bit block with pAngle `203` and a
  prepared left edge of length `w + h`
- **THEN** each output sample is computed from AV2 §7.13.2.8 non-IDIF
  interpolation over `LeftCol` using `Dr_Intra_Derivative[67]`
- **AND** samples whose computed base reaches the prepared-edge limit use the
  edge-end fallback from §7.13.2.8

#### Scenario: Invalid primitive inputs return typed errors before mutation
- **WHEN** the pAngle, edge length, edge sample range, output stride, output
  length, sample type, or arithmetic is invalid for the requested block and bit
  depth
- **THEN** the primitive returns a typed `splot-recon` error without panicking,
  silently clamping invalid input, emitting `decode/*` diagnostics, or mutating
  caller-owned output

### Requirement: One-Sided Directional Angle Exclusions
The primitive SHALL NOT implement full AV2 §7.13.2.1 edge availability or
fallback preparation, §7.13.2.7 directional mode dispatch, angle-delta
derivation, wide-angle mapping, MRL, luma IDIF, middle pAngles `113`, `135`, or
`157`, directional IBP, current-frame workspace edge synthesis, runtime
`splot decode`, reference-decoder invocation, or new crate dependencies.

#### Scenario: Broad directional behavior remains unsupported
- **WHEN** a caller needs a pAngle outside `45`, `67`, or `203`, luma IDIF, MRL,
  middle-angle two-edge prediction, or full `predict_intra()` dispatch
- **THEN** this primitive does not claim support for that behavior
- **AND** the matrix/status documentation keeps the broader intra reconstruction
  and prediction-process rows partial until those behaviors land separately

### Requirement: One-Sided Directional Angle Fuzz Coverage
The existing `recon_intra_prediction_bytes` fuzz target SHALL exercise the new
direct primitive with bounded structured inputs. The target SHALL cover valid
pAngles `45`, `67`, and `203`, unsupported pAngles, short edges, edge sample
range errors, output shape errors, and 8-bit and 10-bit storage paths, while
remaining self-contained and memory-only.

#### Scenario: Directional angle fuzz target is self-contained
- **WHEN** CI fuzz-smoke enumerates `recon_intra_prediction_bytes`
- **THEN** the target covers one-sided directional-angle direct cases without
  adding runtime `splot decode`, filesystem I/O, network access, subprocesses,
  AVM, dav2d, or broad AV2 conformance claims

