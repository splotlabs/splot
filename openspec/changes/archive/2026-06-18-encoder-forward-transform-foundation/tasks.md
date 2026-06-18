## 1. Forward Transform API

- [x] 1.1 Add a private `forward_transform` module with a crate-private 4x4 DCT_DCT DC-only coefficient block type and checked construction from signed residual samples.
- [x] 1.2 Add typed encoder errors for wrong transform input length, unsupported non-uniform residual input, and coefficient arithmetic overflow.
- [x] 1.3 Wire the private module into `splot-encode` without re-exporting it or changing packet output.

## 2. Forward Transform Tests

- [x] 2.1 Add positive tests for zero, positive, and negative uniform 4x4 residual blocks.
- [x] 2.2 Add negative tests for wrong input length, non-uniform residual blocks, and coefficient overflow.
- [x] 2.3 Add a no-op quant/dequant inverse proof through `splot-recon::inverse_transform_2d_outer`.
- [x] 2.4 Preserve an explicit no-packet-output test while forward transform calculation exists.

## 3. Tracking And Verification

- [x] 3.1 Add `ENC-FORWARD-TRANSFORM-FOUNDATION` to the implementation matrix and refresh generated status docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming broad transform, quantization, tile-body, packet, or CLI success behavior.
- [x] 3.3 Run focused encoder tests, OpenSpec validation, feature-status checks, and `cargo xtask ci`.
