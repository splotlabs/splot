## Why

Brick 2 of the forward DCT + quantizer phase. With the real 4x4 DCT_DCT forward
transform landed (`ENC-FORWARD-TRANSFORM-DCT-4X4`), the encoder can now produce
blocks with real non-zero AC coefficients. The quantizer must quantize all 16
coefficients per-coefficient and prove the emitted levels reconstruct through the
decoder dequant — not just the single DC coefficient the flat path exercised.

The existing `QuantizedTransformBlock::dct_dct_4x4_dc_only` quantizer already loops
all 16 coefficients (index 0 with the DC quantizer, the rest with the AC quantizer)
and dequantizes through `splot-recon` — its name was a misnomer (it was never
DC-only). This change renames it to `dct_dct_4x4`, keeps a thin
`dct_dct_4x4_dc_only` alias for the closed loop's current flat pairing, and proves
the per-coefficient quant over a real non-uniform block.

## What Changes

- Add `ENC-FWD-QUANT-PER-COEFF-AC` as a private `splot-encode` encoder-tool feature.
- Rename `QuantizedTransformBlock::dct_dct_4x4_dc_only` → `dct_dct_4x4` (the general
  per-coefficient quantizer; no logic change). Keep `dct_dct_4x4_dc_only` as a thin
  alias delegating to it (the closed loop's current entry point), documented as the
  flat-input pairing — identical operation, not a different one.
- Prove the quantizer over a real 16-coefficient block (the new forward DCT output):
  per-coefficient round-to-nearest levels, the stored dequantized array equals an
  independent `splot-recon` dequantize_block of the levels, the dequant product
  guard holds, and the decoder reconstruction stays close to the source residual at
  low qindex (bounded, not exact).
- No quantization-policy change (still round-to-nearest v0, no deadzone, no RDO), no
  rate-control, no coefficient tokenization, no syntax, no packet output.

For zero-delta luma the DC and AC quantizers coincide, so the index-0 DC / rest AC
selection is structural here; it differentiates levels only with chroma or DC-delta
quantizers (a later brick). The point proven is that real non-zero AC coefficients
now quantize correctly.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the per-coefficient forward quantizer over
  a real 16-coefficient 4x4 DCT_DCT block, proven against the `splot-recon` dequant.

## Impact

- Affected code: `crates/splot-encode/src/quantization.rs` (rename + alias + tests).
  The one caller (`crates/splot-encode/src/closed_loop.rs`) keeps using the
  `dct_dct_4x4_dc_only` alias unchanged.
- Scope (explicitly NOT claimed here): deadzone / RDO quantization policy, rate
  control, chroma quantizer deltas, bit depths other than 8, transform sizes other
  than 4x4, types other than DCT_DCT, coefficient tokenization, packet output.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status / spec coverage.
