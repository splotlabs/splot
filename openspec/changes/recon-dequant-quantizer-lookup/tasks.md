## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-DEQUANT-QUANTIZER-LOOKUP` to the implementation matrix and decoder support matrix.

## 2. Reconstruction Implementation

- [x] 2.1 Add a new scheduler-free `splot-recon` dequantization module without changing crate dependency direction.
- [x] 2.2 Implement the § 7.14.2 `Ac_Qlookup` table, `qlookup` shift extension, `max_quantizer_index` (§ 6.4.1 Table 6.3 `MaxQ`), and `quantizer_value` (`get_q`).
- [x] 2.3 Make every input total and panic-free using `i64` clamp intermediates, reading no frame, segment, or tile state.
- [x] 2.4 Export the public functions and update the crate documentation.

## 3. Tests

- [x] 3.1 Add spec-exact `Ac_Qlookup` and shift-extension tests at the 8-bit and 10-bit `MaxQ` extremes.
- [x] 3.2 Add `quantizer_value` tests for the qindex-0 special case, delta addition, both clamp directions, and input-extreme panic-freedom.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency, concurrency, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-dequant-quantizer-lookup --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
