## Why

`RECON-INVERSE-TRANSFORM-1D` added the § 7.15.2.1 kernel-based 1D inverse
transform. The § 7.15.2 group has two more 1D transforms the § 7.15.4 2D process
invokes: the § 7.15.2.2 inverse Walsh-Hadamard transform (lossless blocks) and
the § 7.15.2.3 inverse identity transform (the Table 7.1 `IDT` type). Both are
small, matrix-free, and pure — the natural next residual-path brick, completing
the § 7.15.2 1D transforms.

## What Changes

- Add Feature ID `RECON-INVERSE-TRANSFORM-MATRIX-FREE`.
- Extend `crates/splot-recon/src/inverse_transform.rs` with:
  - `inverse_walsh_hadamard` — the § 7.15.2.2 4-element lossless butterfly with a
    pre-scaling `shift` (no kernel, no `Clip3`).
  - `inverse_identity_transform` — the § 7.15.2.3 per-sample
    `Clip3(colTx bound, Round2(src * scale, shift))`, sharing the clamp-bound
    helper with the § 7.15.2.1 kernel transform.
- Keep both total and panic-free with `i64` intermediates; the identity transform
  returns a typed `ReconError` on a source/output length mismatch.
- Preserve the current runtime `splot decode` unsupported behavior.
- Add self-contained spec-exact unit tests.
- Update decoder support, feature tracking, roadmap, generated status docs, and
  OpenSpec artifacts.

Non-goals:

- No § 7.15.4.1 `get_identity_scale` derivation (the caller supplies `scale`).
- No § 7.15.3 secondary transform or § 7.15.4 2D inverse transform orchestration
  (`Transform_Shift`, `get_transform_1d_type`, row/column passes, DPCM, sample
  duplication).
- No dequantization, residual addition, tile-syntax decode, runtime decode
  output, hashes, Y4M, or reference refresh.
- No scheduler state in `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records source-backed AV2 § 7.15.2.2 and § 7.15.2.3 inverse
  transform support while broader reconstruction (the § 7.15.4 2D transform,
  dequantization, and residual addition) remains partial.

## Impact

- `crates/splot-recon/src/inverse_transform.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `docs/DECODER-ROADMAP.md`
- `openspec/specs/decoder-support/spec.md`
