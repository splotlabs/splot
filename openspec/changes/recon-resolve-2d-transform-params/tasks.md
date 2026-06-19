## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-RESOLVE-2D-TRANSFORM-PARAMS` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `InverseTransform2dOuter::resolve` deriving `row_shift` / `col_shift` from `transform_shift` and `row_type` / `col_type` from `get_transform_1d_type` over the adjusted per-pass sample sizes, plus `plane_tx_type_is_idtx` from `PlaneTxType == IDTX`.
- [x] 2.2 Resolve every transform-size/type field from one `(plane_tx_type, log2_width, log2_height)` source the result stores, and keep the helper a total, panic-free `const fn` that validates the shape and type before resolving (no new error variant).
- [x] 2.3 Update the crate `//!` implemented/not-implemented lists and feature-tracking enumeration.

## 3. Tests

- [x] 3.1 Add the helper-argument wiring test, the per-pass adjusted-size DDT substitution test, and the end-to-end equivalence-with-manual-params test.
- [x] 3.2 Add the fail-atomic shape/type rejection test, the IDTX-flag test, the totality sweep, and a module-level `const`-evaluated spec contract.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency-direction, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-resolve-2d-transform-params --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
