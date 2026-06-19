## 1. Closed-loop API

- [x] 1.1 Add a private `closed_loop` module that reconstructs the current 8-bit luma 4x4 DCT_DCT DC-only top-left uniform subset through DC intra prediction, residual, forward transform, quantization, dequantization, inverse transform, and reconstruct (residual addition).
- [x] 1.2 Use `splot-recon` for every decoder-visible step (prediction, dequant, inverse transform, residual addition, current-frame workspace, decoded-frame hash) and keep encoder-policy residual/transform/quantization in `splot-encode`.
- [x] 1.3 Freeze the reconstructed block into a `splot-recon` monochrome current-frame workspace and compute its decoded-frame hash.
- [x] 1.4 Add typed encoder errors for unsupported source size, unsupported bit depth, and the prediction/inverse-transform/residual-add/workspace handoffs.
- [x] 1.5 Wire the private module into `splot-encode` without re-exporting it or changing packet output.

## 2. Closed-loop tests

- [x] 2.1 Prove the qindex-zero lossless flat subset reconstructs to the source samples exactly.
- [x] 2.2 Prove reconstruction and the decoded-frame hash are deterministic across repeated runs, and cross-check the hash against an independently built workspace.
- [x] 2.3 Prove the emitted coefficient decisions (tokenization roundtrip through the in-tree AV2 §8.2 symbol coder) decode back to the exact quantized coefficient the closed loop reconstructs from.
- [x] 2.4 Add negative tests for non-uniform source, unsupported source size, and unsupported bit depth.
- [x] 2.5 Preserve an explicit no-packet-output test while closed-loop reconstruction exists.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming tile-body, packet, CLI, reference-store, inter, rate-control, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
