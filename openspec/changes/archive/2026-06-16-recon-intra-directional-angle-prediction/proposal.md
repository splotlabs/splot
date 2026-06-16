## Why

The decoder conformance roadmap still has a gap between H/V cardinal directional
prediction and the broader AV2 directional-angle process. A small, source-backed
one-sided angular primitive gives future tile decode code tested §7.13.2.8 math
for the chroma no-IDIF cases without claiming full directional dispatch.

## What Changes

- Add Feature ID `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`.
- Add a scheduler-free `splot-recon` primitive for already-prepared one-sided
  chroma directional-angle prediction.
- Support pAngles `45`, `67`, and `203`, using AV2 §7.13.2.8 non-IDIF bilinear
  interpolation and the §9.2 derivative entries needed by those angles.
- Validate bit depth, sample type, prepared edge length, edge sample ranges,
  output stride, output length, and arithmetic before writing prediction output.
- Extend the existing intra prediction fuzz target to cover the new primitive's
  valid and typed-error paths.
- Update implementation, decoder-support, generated status, roadmap, and
  conformance coverage docs for the new narrow capability.
- Do not change validator behavior or user-facing diagnostics.

## Capabilities

### New Capabilities

- `recon-intra-directional-angle-prediction`: Source-backed one-sided
  directional-angle intra prediction primitive for the non-IDIF pAngle `45`,
  `67`, and `203` branches of AV2 §7.13.2.8.

### Modified Capabilities

- `decoder-support`: Record the new primitive and keep broad intra
  reconstruction partial until full edge preparation, middle-angle branches,
  luma IDIF, MRL, IBP, and runtime dispatch land.
- `conformance`: Extend self-contained fuzz coverage metadata for the existing
  intra prediction fuzz target.

## Impact

- Affected code: `crates/splot-recon`, `fuzz/fuzz_targets`, `docs`, `xtask`,
  and `openspec`.
- Public API impact: new `splot-recon` types/functions for the narrow
  directional-angle primitive.
- Dependency impact: none. `splot-recon` remains independent of `splot-core`.
- Non-goals: full §7.13.2.1 edge availability/fallback, §7.13.2.7 mode dispatch,
  angle-delta derivation, wide-angle mapping, MRL, IDIF/luma filtering, middle
  angles `113`/`135`/`157`, directional IBP, workspace edge synthesis, runtime
  `splot decode`, AVM/dav2d integration, new dependencies, or encoder behavior.
