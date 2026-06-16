## Why

Before this change, the decoder roadmap listed subsampled DC as a missing
intra-prediction piece for large chroma `DC_PRED`/`UV_CFL_PRED` paths. Adding
the scalar prepared-edge primitive closes a small AV2 §7.13.2.11 gap without
claiming full CfL dispatch, broad `predict_intra()`, or runtime tile decode
support.

## What Changes

- Add a scheduler-free `splot-recon` primitive for AV2 v1.0.0 §7.13.2.11 DC
  intra prediction subsampled process:
  - validate prepared `LeftCol[0..h]` and `AboveRow[0..w]` edge lengths and
    bit-depth ranges;
  - average every sample for dimensions up to 32 and every second sample for
    dimensions greater than 32;
  - use the existing AV2 approximate-division path and midpoint no-edge
    fallback.
- Add current-frame workspace handoff for in-storage subsampled DC prediction
  without deciding AV2 `largeChroma`, `UV_CFL_PRED`, tile-boundary, MRL, or
  block-availability semantics.
- Extend the existing recon intra fuzz target and support/matrix docs for the
  new primitive.
- Non-goals: no full `predict_intra()` dispatch, no CfL luma-subsampling
  process, no data-driven prediction, IBP, general directional prediction,
  transform/residual, loop filters, runtime `splot decode` expansion,
  AVM/dav2d integration, or new dependencies.

## Capabilities

### New Capabilities

- `recon-intra-dc-subsampled-prediction`: Source-backed scalar
  `splot-recon` primitive and workspace handoff for AV2 §7.13.2.11 prepared-edge
  subsampled DC prediction.

### Modified Capabilities

- `decoder-support`: Record Feature ID
  `RECON-INTRA-DC-SUBSAMPLED-PREDICTION`, its tests/fuzz evidence, and the
  unchanged partial status of broad intra reconstruction and prediction-process
  rows.

## Impact

- Affected code: `splot-recon` intra/workspace modules, the existing recon intra
  fuzz target, decoder support/matrix docs, conformance coverage metadata, and
  OpenSpec artifacts.
- Validator impact: none.
- User-facing diagnostics: none added or changed; this is not a byte-consuming
  runtime decoder expansion.
- Dependencies and licensing: no new dependency, no AVM/dav2d invocation, and
  no copied third-party code or tables.
