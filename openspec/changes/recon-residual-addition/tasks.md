## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-RESIDUAL-ADDITION` to the implementation matrix and decoder support matrix.

## 2. Reconstruction Implementation

- [x] 2.1 Add `reconstruct_add_residual` implementing the § 7.14.3 `Clip1(prediction + residual)` step over caller-supplied prediction and residual blocks.
- [x] 2.2 Validate the sample type against the bit depth and equal prediction/residual/output lengths with typed `ReconError`.
- [x] 2.3 Sum with `i64` intermediates so the primitive is total and panic-free; export the public item and update docs.

## 3. Tests

- [x] 3.1 Add a plain add vector and both `Clip1` clamp directions, plus the 10-bit u16 path.
- [x] 3.2 Add the `i32` residual-extreme totality case and the sample-type and length-mismatch error cases.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-residual-addition --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
