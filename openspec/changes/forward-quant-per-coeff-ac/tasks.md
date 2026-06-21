## 1. Implementation

- [x] 1.1 Rename `QuantizedTransformBlock::dct_dct_4x4_dc_only` → `dct_dct_4x4`
      (general per-coefficient quantizer; no logic change).
- [x] 1.2 Keep `dct_dct_4x4_dc_only` as a thin alias delegating to `dct_dct_4x4`
      (the closed loop's flat-input entry point; unchanged caller).

## 2. Tests

- [x] 2.1 Real non-uniform block: every level == round-to-nearest of its coefficient
      at its selected quantizer; non-zero AC levels present.
- [x] 2.2 Stored dequantized == independent `splot-recon` dequantize_block of the
      emitted levels.
- [x] 2.3 Every emitted level dequantizes within the AV2 24-bit product limit.
- [x] 2.4 Decoder reconstruction (dequant + inverse) within a bounded distance of the
      source residual at qindex 0 (not bit-exact).
- [x] 2.5 The `dct_dct_4x4_dc_only` alias equals the general entry point.

## 3. Tracking

- [x] 3.1 Add the `ENC-FWD-QUANT-PER-COEFF-AC` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
