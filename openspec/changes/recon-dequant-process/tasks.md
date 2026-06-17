## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-DEQUANT-PROCESS` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `dequant_coefficient` implementing the § 7.14.4 per-coefficient steps 3-8 (sign, `Abs(qc)*q2`, `Round2(.. & 0xFFFFFF, 3)`, `/ dq_denom`, `Clip3`).
- [x] 2.2 Add `dequantize_block` + `DequantBlockParams` applying it over a `tx_width*tx_height` block with DC/AC quantizer selection (non-quantization-matrix path).
- [x] 2.3 Keep the computation total and panic-free; validate shape and buffer lengths with typed `ReconError`; export the public items and update docs.

## 3. Tests

- [x] 3.1 Add the Round2-mask vector, the dq_denom divide, the 24-bit mask, and both bit-depth clip bounds.
- [x] 3.2 Add the `i32::MIN` / max-quantizer totality extreme, the DC-vs-AC block selection, and the shape/length rejections.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-dequant-process --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
