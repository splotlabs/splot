## Why

The decoder conformance roadmap still leaves the AV2 directional-angle process
partial after the cardinal and one-sided angular slices. A focused middle-angle
primitive covers the remaining non-IDIF prepared-edge branch in AV2 7.13.2.8
without claiming full directional prediction or runtime decode dispatch.

## What Changes

- Add Feature ID `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION`.
- Add a scheduler-free `splot-recon` primitive for already-prepared middle
  directional-angle prediction.
- Support pAngles `113`, `135`, and `157`, using AV2 7.13.2.8 non-IDIF
  bilinear interpolation and the needed AV2 9.2 derivative entries.
- Model the middle branch's signed logical edge indexing explicitly so callers
  provide the required `AboveRow` and `LeftCol` ranges without this primitive
  synthesizing edge availability or fallback samples.
- Validate bit depth, sample type, prepared edge ranges, edge sample ranges,
  output stride, output length, pAngle support, and arithmetic before writing
  prediction output.
- Extend intra prediction fuzz coverage to include valid middle-angle inputs and
  typed-error paths.
- Update implementation, decoder-support, generated status, roadmap, and
  conformance coverage docs for the narrow capability.
- Do not change validator behavior or user-facing decoder diagnostics.

## Capabilities

### New Capabilities

- `recon-intra-middle-directional-angle-prediction`: Source-backed middle
  directional-angle intra prediction primitive for the non-IDIF pAngle `113`,
  `135`, and `157` branch of AV2 7.13.2.8.

### Modified Capabilities

- `decoder-support`: Record the new primitive and keep broad intra
  reconstruction partial until full edge preparation, luma IDIF, MRL,
  directional IBP, and runtime dispatch land.
- `conformance`: Extend self-contained fuzz coverage metadata for the existing
  intra prediction fuzz target.

## Impact

- Affected code: `crates/splot-recon`, `fuzz/fuzz_targets`, `docs`, `xtask`,
  and `openspec`.
- Public API impact: new or extended `splot-recon` types/functions for the
  narrow middle-angle primitive.
- Dependency impact: none. `splot-recon` remains independent of `splot-core`.
- Non-goals: full AV2 7.13.2.1 edge availability/fallback preparation, AV2
  7.13.2.7 mode dispatch, angle-delta derivation, wide-angle mapping, MRL,
  IDIF/luma filtering, pAngles outside `113`/`135`/`157`, directional IBP,
  workspace edge synthesis, runtime `splot decode`, AVM/dav2d integration, new
  dependencies, or encoder behavior.
