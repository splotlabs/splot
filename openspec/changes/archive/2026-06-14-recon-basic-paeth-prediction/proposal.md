## Why

Stage 8 needs the next small source-backed intra prediction primitive after
square and rectangular DC. AV2 §7.13.2.2 basic intra prediction is narrow,
scheduler-free, and directly reusable by future decode and encoder
reconstruction paths for `PAETH_PRED`.

## What Changes

- Add Feature ID `RECON-INTRA-BASIC-PAETH-PREDICTION`.
- Add a scheduler-free `splot-recon` basic/PAETH rectangular prediction
  primitive over caller-provided left, above, and top-left edge samples.
- Preserve the current runtime `splot decode` unsupported behavior.
- Add current-frame workspace helpers only if they can stay allocation-free and
  avoid deciding AV2 block, tile, or edge-availability semantics.
- Add self-contained positive and typed-failure tests.
- Update decoder support, feature tracking, roadmap, and OpenSpec artifacts.

Non-goals:

- No directional, smooth, data-driven, subsampled DC, IBP, CfL, transform,
  dequantization, residual, loop-filter, runtime hash, runtime Y4M, reference
  refresh, or full `predict_intra()` dispatch support.
- No `splot-decode -> splot-recon` dependency.
- No scheduler state in `splot-recon`; future orchestration remains in
  `DecodeContext` and `splot_parallel::WorkerPool`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records source-backed AV2 §7.13.2.2 basic/PAETH intra
  prediction support while broader scalar intra reconstruction remains partial.

## Impact

- `crates/splot-recon/src/intra_basic.rs`
- `crates/splot-recon/src/workspace.rs`
- `crates/splot-recon/src/error.rs` if a new typed input error is needed
- `crates/splot-recon/src/lib.rs`
- `crates/splot-recon/src/workspace_tests.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `docs/DECODER-ROADMAP.md`
