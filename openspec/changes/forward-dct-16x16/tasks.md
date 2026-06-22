## 1. Implementation
- [x] 1.1 `forward_dct16_1d` + `dct_dct_16x16` (transposed `DCT_KERNEL16`, shifts `(0,13)`).
- [x] 1.2 `QuantizedTransformBlock16x16::dct_dct_16x16` (quant + splot-recon dequant).

## 2. Tests
- [x] 2.1 Closed-loop reconstruction over random blocks (`|err|` bound), flat-DC lossless
      anchor, kernel-orientation pin.

## 3. Tracking
- [x] 3.1 Add the `ENC-FORWARD-TRANSFORM-DCT-16X16` matrix row.
- [x] 3.2 Regenerate feature status; run `cargo xtask ci`.
