## Why

The decoder roadmap still lists IBP as a missing intra-reconstruction piece
after the landed DC, subsampled DC, smooth, PAETH, and H/V cardinal primitives.
Adding the narrow AV2 §7.13.2.12 DC modifier closes a source-backed scalar
primitive gap without claiming full `predict_intra()` dispatch, general IBP for
directional modes, or runtime tile decode support.

## What Changes

- Add a scheduler-free `splot-recon` primitive for AV2 v1.0.0 §7.13.2.12 IBP
  DC process, tracked by Feature ID `RECON-INTRA-IBP-DC-PREDICTION`.
- The primitive modifies a caller-owned DC prediction buffer using prepared
  `LeftCol[0..h]` and `AboveRow[0..w]` edges when those edges are available,
  validates edge lengths, bit-depth ranges, sample type, output stride, and
  output length, and uses the spec `Ibp_Weights` table and `Round2` blend.
- Add a current-frame workspace helper that writes in-storage DC prediction and
  then applies the IBP DC modifier from in-storage left and/or above edges.
- Extend the existing recon intra fuzz target and support/matrix docs for the
  new primitive.
- Non-goals: no general directional-angle IBP, no §7.13.2.9 dynamic IBP
  weights, no full §7.13.2.1 `predict_intra()` dispatch, no data-driven
  prediction, no CfL/CCTX/MHCCP or palette, no transform/residual, no loop
  filters, no runtime `splot decode` expansion, no AVM/dav2d integration, and
  no new dependencies.

## Capabilities

### New Capabilities

- `recon-intra-ibp-dc-prediction`: Source-backed scalar `splot-recon` primitive
  and workspace handoff for AV2 §7.13.2.12 prepared-edge IBP DC prediction.

### Modified Capabilities

- `decoder-support`: Record Feature ID `RECON-INTRA-IBP-DC-PREDICTION`, its
  tests/fuzz evidence, and the unchanged partial status of broad intra
  reconstruction and prediction-process rows.
- `conformance`: Extend the existing `recon_intra_prediction_bytes` fuzz target
  requirement to include the new IBP DC direct and workspace cases while keeping
  broad §7.13 conformance partial.

## Impact

- Affected code: `splot-recon` intra/workspace modules, the existing recon intra
  fuzz target, decoder support/matrix docs, conformance coverage metadata, and
  OpenSpec artifacts.
- Validator impact: none.
- User-facing diagnostics: none added or changed; this is not a byte-consuming
  runtime decoder expansion.
- Dependencies and licensing: no new dependency, no AVM/dav2d invocation, and
  no copied reference-software code, comments, or prose. The AV2
  §7.13.2.12 normative numeric `Ibp_Weights` table is encoded as cited
  implementation data.
