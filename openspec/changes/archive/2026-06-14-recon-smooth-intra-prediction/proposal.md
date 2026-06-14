## Why

Stage 8 needs the next narrow source-backed intra prediction primitive after
DC and PAETH. AV2 §7.13.2.13 smooth prediction is small enough for one PR,
scheduler-free, and directly reusable by future decode and encoder
reconstruction paths for `SMOOTH_PRED`, `SMOOTH_V_PRED`, and
`SMOOTH_H_PRED`.

## What Changes

- Add Feature ID `RECON-INTRA-SMOOTH-PREDICTION`.
- Add scheduler-free `splot-recon` smooth rectangular prediction over
  caller-provided prepared `LeftCol` and `AboveRow` edge samples.
- Model the three smooth modes explicitly: `SMOOTH_PRED`,
  `SMOOTH_V_PRED`, and `SMOOTH_H_PRED`.
- Add a current-frame workspace helper only if it can stay policy-free and use
  in-storage left/above plus bottom-left/top-right sentinel samples.
- Preserve the current runtime `splot decode` unsupported behavior.
- Add self-contained positive and typed-failure tests.
- Update decoder support, feature tracking, roadmap, generated status docs, and
  OpenSpec artifacts.

Non-goals:

- No full `predict_intra()` dispatcher.
- No §7.13.2.1 edge availability/fallback preparation, MRL, tile-boundary, or
  superblock semantics.
- No directional, DIP, subsampled DC, IBP, CfL, transform, dequantization,
  residual, loop-filter, runtime hash, runtime Y4M, reference refresh, or
  tile-syntax decode support.
- No `splot-decode -> splot-recon` dependency.
- No scheduler state in `splot-recon`; future orchestration remains in
  `DecodeContext` and `splot_parallel::WorkerPool`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records source-backed AV2 §7.13.2.13 smooth intra
  prediction support while broader scalar intra reconstruction remains partial.

## Impact

- `crates/splot-recon/src/intra_smooth.rs`
- `crates/splot-recon/src/workspace.rs`
- `crates/splot-recon/src/error.rs`
- `crates/splot-recon/src/lib.rs`
- `crates/splot-recon/src/workspace_tests.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `docs/DECODER-ROADMAP.md`
- `openspec/specs/decoder-support/spec.md`
