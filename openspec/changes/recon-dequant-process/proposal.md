## Why

The residual path has the § 7.14.2 quantizer functions and the § 7.15 inverse
transforms, but the § 7.14.4 dequantization process that bridges them — turning
coded `Quant` coefficients into the `Dequant` array the inverse transform
consumes — was missing. It is a clean, self-contained arithmetic step on the
critical path to any residual decode.

## What Changes

- Add Feature ID `RECON-DEQUANT-PROCESS`.
- Add `crates/splot-recon/src/dequant_process.rs` with `dequant_coefficient`
  (the § 7.14.4 per-coefficient steps 3-8) and `dequantize_block` + the
  `DequantBlockParams` carrier (the transform-block helper that selects the DC
  quantizer for the `(0, 0)` coefficient and the AC quantizer otherwise — the
  non-quantization-matrix path).
- Take the per-coefficient quantizer `q2` (the § 7.14.2 DC/AC quantizer,
  optionally quantization-matrix-weighted `Round2(q * m, 5)`) and the dequant
  denominator `dq_denom = 1 << shift` as caller-resolved inputs.
- Keep the computation total and panic-free (`i64` product/rounding,
  `unsigned_abs`, zero `dq_denom` treated as 1, the `Clip3` bound); validate the
  transform shape and the `tx_width * tx_height` buffer lengths with typed
  `ReconError`.
- Preserve the current runtime `splot decode` unsupported behavior.
- Add self-contained spec-exact unit tests.
- Update decoder support, feature tracking, roadmap, generated status docs, and
  OpenSpec artifacts.

Non-goals:

- No § 7.14.4 quantization-matrix weighting (the `Quantizer_Matrix` / `UserQm`
  lookups), no `shift` / `useFsc` / `allow_tcq` derivation, and no adjusted-size
  handling beyond the `Min(32, ·)` block.
- No coefficient entropy decode (which produces `Quant`), no § 7.15.4 inverse
  transform invocation, no residual addition wiring.
- No tile-syntax decode, runtime decode output, hashes, Y4M, or reference
  refresh.
- No scheduler state in `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records source-backed AV2 § 7.14.4 dequantization process
  support (per-coefficient core + the non-quantization-matrix block helper) while
  broader reconstruction (the quantization-matrix weighting, the coefficient
  entropy decode, and the inverse-transform invocation) remains partial.

## Impact

- `crates/splot-recon/src/dequant_process.rs`
- `crates/splot-recon/src/error.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- generated status/coverage docs
- `docs/DECODER-ROADMAP.md`
- `openspec/specs/decoder-support/spec.md`
