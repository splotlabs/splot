## Why

`splot-recon` can already model immutable decoded frames, hash caller-supplied
frames, write Y4M for caller-supplied frames, store immutable references, and
write square DC predictions into caller-owned buffers. The missing encoder-grade
bridge is a checked mutable current-frame workspace that future decode and
encoder paths can fill incrementally before freezing into the existing immutable
frame/hash/Y4M/reference surfaces.

Feature ID: `RECON-CURRENT-FRAME-WORKSPACE`.

## What Changes

- Add a scheduler-free `splot-recon` current-frame workspace for mutable
  reconstruction samples.
- Allocate plane storage from existing `DecodedFrameInfo` geometry using checked
  arithmetic and fallible allocation before exposing mutable samples.
- Expose bounded plane/rectangle/block read and write helpers that validate
  coordinates, dimensions, sample type, bit-depth range, and backing length.
- Provide edge-read helpers for future intra prediction and a convenience path
  that writes the existing square DC intra prediction primitive into workspace
  storage.
- Freeze the workspace into the existing immutable `DecodedFrame<T>` so existing
  `DecodedFrameHashInput`, `DecodedFrameHash`, `Y4mWriter`, and
  `ReferenceFrameStore` tests can prove interoperability.
- Update decoder roadmap/status docs, implementation matrix/status docs, and
  OpenSpec `decoder-support` requirements without claiming runtime decode
  success.

Non-goals:

- No `splot-decode -> splot-recon` dependency edge and no `DecodeContext` API
  change in this PR.
- No runtime `splot decode` success path, hash output, Y4M output, CLI behavior
  change, tile syntax traversal, `decode_tile()`, dequantization, inverse
  transforms, residual generation, loop filtering, output scheduling, AV2
  reference refresh, or encoder implementation.
- No direct Rayon, crossbeam, global pool, ad-hoc threads, worker pools, or
  scheduler state in `splot-recon`; future orchestration remains owned by
  `splot-decode` through `DecodeContext` and `splot_parallel::WorkerPool`.
- No AVM/dav2d source, snippets, binaries, submodules, dependencies, wrappers,
  scripts, CI jobs, required `xtask` commands, or mandatory tests.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: records `RECON-CURRENT-FRAME-WORKSPACE` as a supported
  `splot-recon` mutable current-frame workspace and clarifies that full scalar
  intra reconstruction and runtime decode output remain partial/planned.

## Impact

- Code: `crates/splot-recon` public API, typed errors, and unit tests.
- Docs: decoder roadmap, decoder support matrix/status, implementation matrix,
  generated feature/spec status docs, and OpenSpec `decoder-support` spec.
- APIs: adds narrow `splot-recon` workspace types/functions; no `splot-decode`,
  `DecodeContext`, `splot-cli`, or `splot-encode` behavior change.
- Dependencies: no new dependencies and no `splot-*` dependency graph change.
- Diagnostics: no new emitted `decode/*` diagnostics; existing runtime
  `decode/unsupported-feature` remains unchanged.
