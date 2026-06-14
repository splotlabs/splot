## Why

The decoder roadmap's only remaining `todo` tier1 row is broad scalar intra
reconstruction, but implementing the whole §7.13-§7.15 path in one PR would
combine tile syntax, prediction, dequantization, inverse transforms, and runtime
output. The next encoder-useful step is a narrow, source-backed DC intra
prediction primitive in `splot-recon` that future decode orchestration can call
without adding scheduler state.

Feature ID: `RECON-INTRA-DC-SQUARE-PREDICTION`.

## What Changes

- Add a public, scheduler-free `splot-recon` primitive for the square-block
  subset of AV2 §7.13.2.10 DC intra prediction.
- Add typed square block-size and prediction-output values that derive
  `w = h = 1 << log2_size`, validate edge availability/lengths, validate sample
  ranges against `BitDepth`, and fill the predicted block without panics.
- Keep all runtime decode, tile traversal, transform/dequant, inverse transform,
  residual addition, output ordering, hashes, Y4M output, and reference refresh
  out of scope.
- Update decoder roadmap/status docs, implementation matrix/status docs, and
  OpenSpec `decoder-support` requirements to record the first supported scalar
  intra prediction primitive without claiming full reconstruction.
- Add self-contained unit tests for both-edge square DC, left-only, above-only,
  no-edge, invalid block-size, mismatched edge length, unsupported sample
  type/bit-depth, out-of-range samples, and checked allocation behavior.

Non-goals:

- No rectangular DC prediction until a later change models the full
  `resolve_divisor` lookup-table path required by AV2 §7.13.3.22.
- No full `predict_intra()` dispatcher, directional/basic/smooth/DIP/CfL/MHCCP
  prediction, palette prediction, transform block syntax, dequantization,
  inverse transforms, residual addition, loop filters, or `decode_tile()`.
- No byte-consuming runtime decode success path and no change to `splot decode`
  unsupported behavior.
- No new crate dependency and no change to the `splot-*` dependency graph.
- No direct Rayon, crossbeam, global pool, ad-hoc threads, or scheduler state in
  `splot-recon`; future orchestration remains owned by `DecodeContext` and
  `splot_parallel::WorkerPool`.
- No AVM/dav2d source, snippets, binaries, submodules, dependencies, wrappers,
  scripts, CI jobs, required `xtask` commands, or mandatory tests.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: records `RECON-INTRA-DC-SQUARE-PREDICTION` as a supported
  `splot-recon` scalar square DC intra prediction primitive under the existing
  decoder support model, while leaving full scalar intra reconstruction
  partial/planned.

## Impact

- Code: `crates/splot-recon` public API and unit tests only.
- Docs: decoder roadmap, decoder support matrix/status, implementation matrix,
  generated feature/spec status docs, and OpenSpec `decoder-support` spec.
- APIs: adds narrow `splot-recon` types/functions; no `splot-decode`,
  `DecodeContext`, or CLI behavior change.
- Dependencies: no new dependencies and no dependency-direction change.
- Diagnostics: no new emitted `decode/*` diagnostics; existing runtime
  `decode/unsupported-feature` remains unchanged.
