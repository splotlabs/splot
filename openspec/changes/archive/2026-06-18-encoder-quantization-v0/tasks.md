## 1. Quantization API

- [x] 1.1 Add a private `quantization` module with fixed qindex parameters and a crate-private 4x4 quantized block type.
- [x] 1.2 Add typed encoder errors for qindex range, invalid dequant denominator, coefficient range, quantization overflow, and dequant handoff failures.
- [x] 1.3 Wire the private module into `splot-encode` without re-exporting it or changing packet output.

## 2. Quantization Tests

- [x] 2.1 Add positive tests for zero, positive, and negative DC-only coefficient quantization at qindex zero.
- [x] 2.2 Add deterministic rounding and monotonicity tests for non-zero fixed qindex values.
- [x] 2.3 Add negative tests for out-of-range qindex, zero denominator, coefficient range rejection, and dequant-product overflow rejection.
- [x] 2.4 Add a dequant-plus-inverse proof through `splot-recon::dequantize_block` and `splot-recon::inverse_transform_2d_outer`.
- [x] 2.5 Preserve an explicit no-packet-output test while quantization calculation exists.

## 3. Tracking And Verification

- [x] 3.1 Add `ENC-QUANTIZATION-V0` to the implementation matrix and refresh generated status docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming tokenization, tile-body, packet, CLI, rate-control, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run focused encoder tests, OpenSpec validation, feature-status checks, and `cargo xtask ci`.
