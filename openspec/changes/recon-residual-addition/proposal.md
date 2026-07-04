## Why

With the dequant quantizer functions and all three § 7.15.2 1D inverse
transforms in place, the next residual-path step is the § 7.14.3 reconstruct
addition: adding the inverse-transform residual to the predicted samples and
clamping with § 4.8 `Clip1`. It is small, pure, and independent of how the
residual is produced, so it is a clean self-contained brick — the "residual
addition" item on the decoder roadmap.

## What Changes

- Add Feature ID `RECON-RESIDUAL-ADDITION`.
- Add `crates/splot-recon/src/reconstruct.rs` with `reconstruct_add_residual`,
  implementing the § 7.14.3 step
  `CurrFrame[plane][y + i][x + j] = Clip1(CurrFrame + Residual[i][j])` over a
  caller-supplied prediction block and residual.
- Validate the sample type against the bit depth and equal
  prediction/residual/output lengths (typed `ReconError`); sum with `i64`
  intermediates so it is total and panic-free.
- Preserve the current runtime `splot decode` unsupported behavior.
- Add self-contained spec-exact unit tests.
- Update decoder support, feature tracking, roadmap, generated status docs, and
  OpenSpec artifacts.

Non-goals:

- No § 7.15.4 2D inverse transform (which fills the `Residual` array), § 7.14.4
  dequantization process, § 7.15.3 secondary transform, § 7.14.3 DPCM
  adjustment, or lossless-conformance requirement.
- No prediction-sample production or current-frame workspace integration.
- No tile-syntax decode, runtime decode output, hashes, Y4M, or reference
  refresh.
- No scheduler state in `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records source-backed AV2 § 7.14.3 residual-addition support
  while broader reconstruction (the § 7.15.4 2D transform, dequantization, and
  prediction/workspace integration) remains partial.

## Impact

- `crates/splot-recon/src/reconstruct.rs`
- `crates/splot-recon/src/error.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `openspec/specs/decoder-support/spec.md`
