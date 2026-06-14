## Why

The decoder roadmap has source-backed square DC intra prediction, but rectangular
DC prediction is still a documented gap before scalar intra reconstruction can
serve encoder roundtrips. Adding the rectangular subset keeps reconstruction
progress moving inside `splot-recon` without wiring runtime `splot decode`
success or changing the decoder concurrency boundary.

## What Changes

- Add scheduler-free rectangular DC intra prediction primitives in
  `splot-recon`, tracked by `RECON-INTRA-DC-RECTANGULAR-PREDICTION`.
- Reuse the existing decoded-frame bit-depth/sample validation and workspace
  patterns while adding rectangular block geometry, edge validation, and checked
  output writes.
- Add current-frame workspace convenience helpers for rectangular DC prediction
  over in-storage left/above edges.
- Update decoder support docs, generated status, and the implementation matrix
  to record rectangular DC as supported and broad intra reconstruction as still
  partial.
- Non-goals: no non-DC intra modes, no `predict_intra()` dispatch, no tile
  syntax traversal, no dequantization, no inverse transforms, no residual
  addition, no runtime frame hashes/Y4M, no AVM/dav2d integration, no scheduler
  state in `splot-recon`, and no `splot-decode -> splot-recon` dependency.

## Capabilities

### New Capabilities

### Modified Capabilities

- `decoder-support`: add rectangular DC intra prediction requirements and
  update the current-frame workspace reconstruction requirement to include
  rectangular DC helper behavior while keeping runtime decode unsupported.

## Impact

- Affected code: `crates/splot-recon/src/intra.rs`,
  `crates/splot-recon/src/workspace.rs`,
  `crates/splot-recon/src/error.rs`, and `crates/splot-recon/src/lib.rs`.
- Affected docs/status: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/DECODER-SUPPORT-STATUS.md`,
  `docs/IMPLEMENTATION-MATRIX.toml`, and the OpenSpec decoder-support delta.
- API impact: additive `splot-recon` public APIs only; no breaking changes and
  no new dependencies.
- Validator/CLI impact: none. `splot decode` remains unsupported at runtime for
  decoded output.
