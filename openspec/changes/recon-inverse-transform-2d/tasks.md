## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-INVERSE-TRANSFORM-2D` to the implementation matrix and decoder support matrix.

## 2. Reconstruction Implementation

- [x] 2.1 Add `inverse_transform_2d` with `InverseTransform2d` / `InverseTransform2dDim` implementing the § 7.15.4.1 row-then-column 2D matrix transform over caller-supplied dequantized blocks.
- [x] 2.2 Carry the original (unadjusted) `txSz` log2 dimensions; derive the adjusted operating size as `1 << Min(log2, 5)`; compute the √2 rescale parity and per-pass `get_identity_scale` from the original log2 dimensions.
- [x] 2.3 Validate the log2 shape and `w * h` buffer lengths with typed `ReconError`; use fixed 32x32 stack buffers and the total 1D primitives so the transform is panic-free for valid shapes; export the public items and update docs.

## 3. Tests

- [x] 3.1 Add DC-only DCT flat-field vectors (4x4 and 8x8), the lossless 4x4 Walsh-Hadamard vector, identity position preservation, and the rectangular 4x8 rescale path.
- [x] 3.2 Add the original-vs-adjusted parity regression (TX_64X32 vs a pre-rescaled 32x32), a mixed row-DCT/column-identity energy-confinement case, the `round2_2896` √2 vector, and the shape/lossless/buffer rejection cases.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-inverse-transform-2d --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
