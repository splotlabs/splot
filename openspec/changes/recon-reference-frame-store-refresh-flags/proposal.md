## Why

The source-backed `splot-recon` reference-frame store can already hold caller
payloads by slot, but future decode and encoder-reuse code also needs a safe
way to apply an already-derived AV2 refresh mask without duplicating payloads or
claiming full §7.23 reference-state semantics.

## What Changes

- Update Feature ID `RECON-REFERENCE-FRAME-STORE` to cover a storage-only
  refresh-mask helper.
- Add a typed `ReferenceRefreshMask` for the AV2 §3 `NUM_REF_FRAMES == 16`
  slot ceiling.
- Add a `ReferenceFrameStore<F>` helper that validates a caller-supplied mask
  against the store capacity before mutation, then stores one caller-produced
  payload handle per selected slot in ascending slot order.
- Return replaced payloads without requiring `F: Clone`.
- Extend focused unit tests and the existing
  `recon_reference_frame_store_bytes` fuzz target for mask construction,
  preflight failures, zero-mask no-ops, and multi-slot refresh behavior.
- Update decoder support, conformance metadata, generated status docs, and
  roadmap/testing notes without broadening runtime decode support.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: Record the storage-only refresh-mask helper under the
  existing `reference-frame-store` row while keeping AV2 reference validity,
  output scheduling, metadata, CDF, grain, motion, and byte-consuming decode
  semantics out of scope.
- `conformance`: Extend the existing reference-frame-store fuzz target contract
  so arbitrary operation sequences also exercise `ReferenceRefreshMask` and the
  multi-slot storage helper.

## Impact

- Affected code: `crates/splot-recon/src/reference.rs`,
  `crates/splot-recon/src/error.rs`, `crates/splot-recon/src/lib.rs`, and
  `fuzz/fuzz_targets/recon_reference_frame_store_bytes.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs,
  `docs/DECODER-ROADMAP.md`, and `docs/TESTING.md`.
- Dependencies: no new third-party dependency and no new `splot-*` dependency
  edge.
- Runtime behavior: no `splot decode` behavior change.
- Validator impact: no new validator diagnostic; this is a reconstruction
  storage API addition with typed `ReconError` failures.
- External tools: no AVM, dav2d, ffmpeg, filesystem output, network, or
  subprocess use.
