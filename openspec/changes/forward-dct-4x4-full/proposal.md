## Why

The maintainer chose the lossy real-encoder path: build a real forward DCT +
quantizer rather than inverting the decoder dequant for exact reconstruction. The
current `forward_transform.rs` is a flat-only stub — it maps a *uniform* 4x4
residual to a single DC coefficient (`v * 32`) and rejects everything else — so it
cannot transform the AC content of a real (non-uniform) intra block.

This change adds the real 16-coefficient 4x4 DCT_DCT forward transform, the first
brick of the forward-transform phase. AV2 specifies only the *inverse* transform,
so the forward transform is derived as its exact numerical inverse: the transposed
§ 9 `DCT_KERNEL4` applied as a row pass then a column pass, with down-shifts that
pair with the decoder's § 7.15.4 4x4 inverse shifts. It is proven by round-tripping
through the existing `splot-recon` inverse transform used as the oracle.

## What Changes

- Add `ENC-FORWARD-TRANSFORM-DCT-4X4` as a private `splot-encode` encoder-tool
  feature.
- Add `pub(crate)` `ForwardTransformBlock::dct_dct_4x4(plane, block, residual)` in
  `forward_transform.rs`: it maps **any** signed 4x4 residual (uniform or not) to
  all 16 row-major coefficients via the transposed § 9 `DCT_KERNEL4` (a row pass at
  `FORWARD_ROW_SHIFT = 0` then a column pass at `FORWARD_COL_SHIFT = 11`; the two
  shifts sum to 11, the bit budget the § 7.15.4 inverse removes vs the forward 2D
  DCT gain). Passes accumulate in `i64`; the final coefficient is a checked `i32`
  narrowing (a typed error, never a wrap/panic, for out-of-domain residuals).
- The existing flat `dct_dct_4x4_dc_only` is left unchanged (the closed loop still
  uses it); a test proves the full DCT reproduces it bit-exactly on uniform input.
- Re-export `DCT_KERNEL4` from `splot-recon` (it already depends on `splot-tables`)
  so the encoder forward transform consumes the same single generated § 9 kernel
  the decoder inverse uses — no crate dependency-graph change.
- Add the `ForwardTransformCoefficientRangeExceeded` typed error.
- No quantization change, no transform selection, no syntax, no packet output.

The forward transform is bit-exact against the decoder inverse only for the flat
(DC-only) subset. General AC content reconstructs within a small bound (observed
`<= 5` over the tested 8-bit residual domain), **not** bit-exactly, because the AV2
integer DCT4 odd basis rows are not orthonormal; later quantization absorbs this
residue.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the real 4x4 DCT_DCT forward transform
  (all 16 coefficients) derived as the numerical inverse of the § 7.15.4 4x4
  inverse transform.

## Impact

- Affected code: `crates/splot-encode/src/forward_transform.rs`,
  `crates/splot-encode/src/error.rs`, `crates/splot-recon/src/lib.rs` (re-export).
- Scope (explicitly NOT claimed here): forward quantization, transform/size/type
  selection, transform sizes other than 4x4, transform types other than DCT_DCT,
  chroma, bit depths other than 8, coefficient tokenization, packet output, CLI
  success, rate control, and Baseline Encoder Profile v1 output. The 8-bit residual
  bound is empirical over the tested domain, not a proven worst case.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status / spec coverage, and the encoder roadmap.
