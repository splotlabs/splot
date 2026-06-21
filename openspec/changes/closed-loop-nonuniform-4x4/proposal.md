## Why

Brick 3 of the forward DCT + quantizer phase. With the real forward DCT (#412) and
per-coefficient quantizer (#416) landed, the closed loop can now reconstruct a
**non-uniform** intra block end to end: predict → residual → real 16-coefficient
forward DCT → per-coefficient quant → `splot-recon` dequant → inverse transform →
residual add. Until now the closed loop only ran the flat (uniform-residual) path,
because its forward call used the flat-only transform.

## What Changes

- Add `ENC-CLOSED-LOOP-NONUNIFORM-4X4` as a private `splot-encode` encoder-tool
  feature.
- Add `MinimalClosedLoopReconstruction::reconstruct_luma_4x4(source, params)`: the
  general entry point that uses the real 16-coefficient forward DCT, so a
  non-uniform source produces real AC coefficients. The quantized decisions
  reconstruct entirely through `splot-recon` (no encoder-side reconstruction math).
- Refactor the shared pipeline into `prepare` (bit-depth/size validation, DC
  prediction, residual) and `finish` (quant + reconstruct + hash); both entry
  points reuse them and differ only in the forward-transform call.
- Keep `reconstruct_luma_4x4_dc_only` as the flat entry point (the closed loop's
  existing pairing); it still uses the flat-only forward transform, so a non-uniform
  residual is rejected with `ForwardTransformNonUniformResidual` (unchanged).
- Rename the now-general internal helpers honestly:
  `reconstruct_dc_only_from_quantized` → `reconstruct_from_quantized` and
  `dequantize_dc_only` → `dequantize_block_4x4` (both already processed all 16
  coefficients; the `dc_only` names were misnomers).
- Fold in the #416 review nit: drop a stray `§` from a `quantization.rs` doc
  comment.
- No coefficient tokenization, no packet output, no chroma/inter/multi-block.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for closed-loop reconstruction of a non-uniform
  4x4 luma intra block through the real forward DCT + per-coefficient quantizer.

## Impact

- Affected code: `crates/splot-encode/src/closed_loop.rs` (entry point + refactor +
  renames + tests), `crates/splot-encode/src/quantization.rs` (doc nit).
- Scope (explicitly NOT claimed here): coefficient tokenization of multi-coefficient
  levels (the §8.2 roundtrip proof for a non-DC block is a later brick), packet
  output, chroma, inter, multi-block, non-4x4, bit depths other than 8, deadzone/RDO
  quantization. The near-lossless reconstruction is bounded, not bit-exact.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status / spec coverage.
