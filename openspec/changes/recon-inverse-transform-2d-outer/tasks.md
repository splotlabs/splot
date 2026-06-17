## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-INVERSE-TRANSFORM-2D-OUTER` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `inverse_transform_2d_outer` with `InverseTransform2dOuter` / `DpcmDirection`, wrapping `inverse_transform_2d` and deriving the adjusted and original sizes from the original log2 dims.
- [x] 2.2 Implement the lossless IDTX bit-shift shortcut, the DPCM cumulative sum (vertical/horizontal via `wrapping_add`), and the sample duplication expanding the adjusted block into the original-size residual.
- [x] 2.3 Validate the log2 shape and adjusted-`dequant` / original-`residual` buffer lengths with typed `ReconError`; export the public items and update docs.

## 3. Tests

- [x] 3.1 Add a no-adjustment-equals-core case, the lossless IDTX shortcut vector, and vertical and horizontal DPCM running-sum vectors.
- [x] 3.2 Add 64-wide and 64x64 sample-duplication cases (vs the adjusted core), a DPCM totality extreme, and the shape and buffer rejection cases.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency, zero-copy, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-inverse-transform-2d-outer --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
