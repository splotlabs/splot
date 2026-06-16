## Why

Phase 9 needs fuzz coverage for source-backed reconstruction/reference storage
boundaries. The repository has unit tests for `ReferenceSlot` and
`ReferenceFrameStore<F>`, but there is no cargo-fuzz target that repeatedly
drives the generic reference-frame store API with arbitrary operation sequences.

## What Changes

- Add Feature ID `CONF-RECON-REFERENCE-FRAME-STORE-FUZZ`.
- Add a cargo-fuzz target named `recon_reference_frame_store_bytes`.
- Drive the public `splot-recon` `ReferenceSlot` and `ReferenceFrameStore<F>`
  APIs with a bounded state-machine grammar over small non-Clone payloads.
- Check capacity validation, slot construction, bounds-checked `contains_slot`,
  `get`, `put`, and `take`, replacement return paths, `clear`, occupancy, and
  ascending `entries` iteration against a simple oracle model.
- Update support/status docs, testing docs, implementation matrix, and decoder
  conformance coverage metadata.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `conformance`: Add no-panic fuzz coverage for the source-backed
  reference-frame store storage API with bounded arbitrary operation sequences.
- `decoder-support`: Track reference-frame store fuzz coverage as a scoped row
  without changing broad runtime decode, reference refresh, output scheduling,
  or resource-diagnostic claims.

## Impact

- Affected code: `fuzz/Cargo.toml` and
  `fuzz/fuzz_targets/recon_reference_frame_store_bytes.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs,
  `docs/TESTING.md`, `AGENTS.md`, `.github/workflows/ci.yml`, and
  `xtask/src/decoder_conformance_coverage.rs`.
- Dependencies: no new third-party dependency and no new `splot-*` dependency
  edge.
- Runtime behavior: no `splot decode` behavior change.
- External tools: no AVM, dav2d, ffmpeg, filesystem output, network, or
  subprocess use.
