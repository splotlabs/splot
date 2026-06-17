## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-INVERSE-TRANSFORM-MATRIX-FREE` to the implementation matrix and decoder support matrix.

## 2. Reconstruction Implementation

- [x] 2.1 Add `inverse_walsh_hadamard` per § 7.15.2.2 (the 4-element lossless butterfly with a pre-scaling shift).
- [x] 2.2 Add `inverse_identity_transform` per § 7.15.2.3 (`Clip3(colTx bound, Round2(src * scale, shift))`), reusing the shared clamp-bound helper.
- [x] 2.3 Keep both total and panic-free with `i64` intermediates; return a typed `ReconError` on identity length mismatch; export the public items and update docs.

## 3. Tests

- [x] 3.1 Add spec-exact Walsh-Hadamard butterfly vectors plus the pre-shift case.
- [x] 3.2 Add the identity scale/round/clamp vector, both `colTx` clamp ranges, and the length-mismatch error.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-transform-matrix-free --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
