## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-DEBLOCK-ADAPTIVE-STRENGTH` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `deblock_side_threshold_index` (the § 7.17.5 `qInd` `const fn`) and `deblock_adaptive_filter_strength` (the `(qThr, side)` derivation reusing `quantizer_value`).
- [x] 2.2 Take `lvl` and the caller-resolved `side_threshold` as facts; keep both total and panic-free with no new error variant; export the items and update the crate and module `//!` docs.

## 3. Tests

- [x] 3.1 Add a `qInd` clip test over both bit depths.
- [x] 3.2 Add a strength test pinning the `side` arithmetic by hand and the `qThr` composition against the independently-tested `quantizer_value`.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency-direction, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-deblock-adaptive-strength --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
