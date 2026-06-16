# recon-intra-middle-directional-angle-prediction Specification

## Purpose
TBD - created by archiving change recon-intra-middle-directional-angle-prediction. Update Purpose after archive.
## Requirements
### Requirement: Middle Directional Angle Primitive
`splot-recon` SHALL provide a scheduler-free scalar primitive for the
source-backed middle directional-angle cases from AV2 v1.0.0 7.13.2.8, tracked
by Feature ID `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION`. The primitive
SHALL accept an active bit depth, rectangular block size, explicit supported
pAngle, prepared logical left and above edge ranges, caller-owned strided output
storage, and SHALL validate sample type, pAngle support, logical edge coverage,
edge sample range, output stride, output length, and checked arithmetic before
reporting success.

#### Scenario: Middle angle predicts from above and left edges
- **WHEN** a caller predicts an 8-bit or 10-bit block with pAngle `113`, `135`,
  or `157` and prepared logical above and left edge ranges covering every
  referenced `base` and `base + 1` position
- **THEN** each output sample is computed from AV2 7.13.2.8 non-IDIF
  interpolation using the AV2 9.2 derivative entries for `180 - pAngle` and
  `pAngle - 90`
- **AND** samples whose first middle-branch base is at least `-1` read from
  `AboveRow`
- **AND** samples below that branch threshold read from `LeftCol`

#### Scenario: Invalid middle-angle inputs return typed errors before mutation
- **WHEN** the pAngle, logical edge coverage, edge sample range, output stride,
  output length, sample type, or arithmetic is invalid for the requested block
  and bit depth
- **THEN** the primitive returns a typed `splot-recon` error without panicking,
  silently clamping invalid input, emitting `decode/*` diagnostics, or mutating
  caller-owned output

### Requirement: Middle Directional Angle Exclusions
The primitive SHALL NOT implement full AV2 7.13.2.1 edge availability or
fallback preparation, AV2 7.13.2.7 directional mode dispatch, angle-delta
derivation, wide-angle mapping, MRL, luma IDIF, one-sided pAngles, pAngles `90`
or `180`, directional IBP, current-frame workspace edge synthesis, runtime
`splot decode`, reference-decoder invocation, or new crate dependencies.

#### Scenario: Broad directional behavior remains partial
- **WHEN** a caller needs edge preparation, IDIF, MRL, mode dispatch, angle
  deltas, directional IBP, runtime decode integration, or a pAngle outside
  `113`, `135`, or `157`
- **THEN** this primitive does not claim support for that behavior
- **AND** the matrix/status documentation keeps broader intra reconstruction and
  prediction-process rows partial until those behaviors land separately

### Requirement: Middle Directional Angle Fuzz Coverage
The existing `recon_intra_prediction_bytes` fuzz target SHALL exercise the new
direct primitive with bounded structured inputs. The target SHALL cover valid
pAngles `113`, `135`, and `157`, unsupported pAngles, missing or short logical
edge ranges, edge sample range errors, output shape errors, and 8-bit and
10-bit storage paths, while remaining self-contained and memory-only.

#### Scenario: Directional middle-angle fuzz target is self-contained
- **WHEN** CI fuzz-smoke enumerates `recon_intra_prediction_bytes`
- **THEN** the target covers middle directional-angle direct cases without
  adding runtime `splot decode`, filesystem I/O, network access, subprocesses,
  AVM, dav2d, or broad AV2 conformance claims

