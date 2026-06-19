## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-DEBLOCK-FILTER-MAX-WIDTH` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `deblock_filter_max_width` implementing the § 7.17.3 branching derivation, as a total `const fn` over caller-resolved scalars.
- [x] 2.2 Add a module-level `const`-evaluated spec contract; export the item and update the crate and module `//!` docs.

## 3. Tests

- [x] 3.1 Add a branch-coverage test over every `filter_size` bucket, both planes, and the super-block-edge cap.
- [x] 3.2 Run focused `splot-recon` tests plus clippy, doc, dependency-direction, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-deblock-filter-max-width --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
