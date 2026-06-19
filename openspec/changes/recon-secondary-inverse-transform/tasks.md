## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-SECONDARY-INVERSE-TRANSFORM` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `secondary_inverse_transform` and `SecondaryInverseTransform` in a new `secondary_transform.rs`, gathering the 2D-scan coefficients, multiplying by the § 9.7 IST kernel, and scattering with `Round2Signed` / `Clip3` and `transpose`.
- [x] 2.2 Hand-write the spec-inline `Stx_Scan_Order_4x4` / `Stx_Scan_Order_8x8` constants; reuse the `splot-tables` IST kernels, `Stx_Scan_Map`, and `coefficient_scan_order`.
- [x] 2.3 Keep it total and panic-free with three new typed `ReconError` variants validated before mutation; export the items and update the crate `//!` docs.

## 3. Tests

- [x] 3.1 Add the `Round2Signed` both-signs test and the hand-computed single-DC test against literal IST kernel values.
- [x] 3.2 Add small-4x4 and large-8x8 reference matches, transpose, the reduced 8x8 height case, fail-atomic rejection, and an i32-extreme totality sweep.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency-direction, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-secondary-inverse-transform --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
