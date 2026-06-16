## Why

Phase 9 needs fuzz coverage for source-backed decoded-frame model boundaries.
The repository has unit tests and downstream hash/Y4M fuzz targets that build
valid `DecodedFrame<T>` values, but no cargo-fuzz target repeatedly drives the
frame and plane runtime type validators, borrowed views, and shared-frame
accessors with arbitrary bounded geometry and sample inputs.

## What Changes

- Add Feature ID `CONF-RECON-FRAME-PLANE-TYPES-FUZZ`.
- Add a cargo-fuzz target named `recon_frame_plane_types_bytes`.
- Drive public `splot-recon` frame/plane model APIs with bounded structured
  inputs derived from arbitrary bytes.
- Exercise AV2-derived bit-depth and chroma-idc mapping, positive geometry,
  crop bounds and alignment, stride and backing-length validation, visible-row
  slicing, frame plane presence and size checks, sample-range checks, borrowed
  `PlaneRef`/`FrameRef` views, and explicit `SharedFrame` sharing.
- Update support/status docs, testing docs, implementation matrix, and decoder
  conformance coverage metadata.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `conformance`: Add no-panic fuzz coverage for the source-backed
  decoded-frame and plane runtime type validators and accessors.
- `decoder-support`: Track decoded-frame/plane fuzz coverage as a scoped row
  without changing broad runtime decode, reconstruction, output scheduling,
  reference refresh, or resource-diagnostic claims.

## Impact

- Affected code: `fuzz/Cargo.toml` and
  `fuzz/fuzz_targets/recon_frame_plane_types_bytes.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs,
  `docs/TESTING.md`, `AGENTS.md`, `.github/workflows/ci.yml`, and
  `xtask/src/decoder_conformance_coverage.rs`.
- Dependencies: no new third-party dependency and no new `splot-*` dependency
  edge.
- Runtime behavior: no `splot decode` behavior change.
- External tools: no AVM, dav2d, ffmpeg, filesystem output, network, or
  subprocess use.
