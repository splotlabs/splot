## Why

The minimal-working-encoder geometry unlock (maintainer-approved Route B: a 16×16-leaf
partition tree). The encoder's only real forward transform was 4×4, but the splot-decode
general-intra runtime rejects sub-8×8 — the smallest decoder-verified square is 16×16. This
adds the real 16×16 forward DCT_DCT + quantizer so the encoder can transform a size the
decoder accepts.

## What Changes

- Add `ENC-FORWARD-TRANSFORM-DCT-16X16` as a private `splot-encode` encoder-tool feature.
- `ForwardTransformBlock16x16::dct_dct_16x16` (transposed §9 `DCT_KERNEL16`, forward shifts
  `(0, 13)` — derived `32 − 19` from the §7.15.4 16×16 inverse, empirically pinned).
- `QuantizedTransformBlock16x16::dct_dct_16x16` (per-coeff DC/AC quant, splot-recon dequant).
- A closed-loop reconstruction test (forward → quant → dequant → §7.15.4 inverse).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: a real 16×16 forward DCT_DCT + quantizer (the decoder-verified size).

## Impact

- Affected code: new `crates/splot-encode/src/{forward_transform_16x16.rs,
  quantization_16x16.rs}` (re-exported from the 4×4 siblings); `lib.rs`.
- Scope (explicitly NOT claimed): transform/size/type selection, sizes other than 16×16,
  non-DCT_DCT, chroma, non-8-bit, tokenization, packet output, a per-block driver.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status.
