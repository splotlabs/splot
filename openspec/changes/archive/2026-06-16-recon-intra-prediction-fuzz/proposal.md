## Why

Phase 9 requires fuzz coverage for every byte-consuming or structured-input
decode stage. The repository now has fuzz coverage for decode planning, minimal
runtime hash output, and Y4M serialization, but existing `splot-recon` intra
prediction and current-frame workspace primitives are still covered only by
unit tests.

## What Changes

- Add Feature ID `CONF-RECON-INTRA-PREDICTION-FUZZ`.
- Add a cargo-fuzz target named `recon_intra_prediction_bytes`.
- Normalize arbitrary bytes into small valid intra prediction cases for the
  existing DC, PAETH, smooth, and current-frame workspace APIs.
- Exercise direct caller-supplied prediction buffers and workspace edge
  extraction/prediction paths with typed return/no panic behavior.
- Keep dimensions, output buffers, workspace planes, sample values, and
  operation counts bounded for CI fuzz smoke.
- Update support/status docs, testing docs, implementation matrix, and decoder
  conformance coverage metadata.

## Capabilities

### New Capabilities

### Modified Capabilities

- `conformance`: Add no-panic fuzz coverage for source-backed intra prediction
  and current-frame workspace reconstruction primitives over bounded structured
  inputs.
- `decoder-support`: Track intra prediction fuzz coverage as a scoped row
  without broadening runtime decode, full intra reconstruction, directional
  prediction, residual, transform, loop-filter, or AVM/dav2d claims.

## Impact

- Affected code: `fuzz/Cargo.toml` and
  `fuzz/fuzz_targets/recon_intra_prediction_bytes.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs,
  `docs/TESTING.md`, and `xtask/src/decoder_conformance_coverage.rs`.
- Dependencies: no new third-party dependency; reuse the existing out-of-
  workspace fuzz crate `splot-recon` path dependency.
- Runtime behavior: no `splot decode` behavior change.
- External tools: no AVM, dav2d, ffmpeg, filesystem output, network, or
  subprocess use.
