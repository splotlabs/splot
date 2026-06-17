## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION` to the implementation matrix and decoder support matrix.

## 2. Reconstruction Implementation

- [x] 2.1 Add `quantizer_index` implementing § 7.14.2 `get_qindex` over caller-resolved facts (segment ALT_Q active/data, base_q_idx, CurrentQIndex, delta_q_present, ignoreDeltaQ).
- [x] 2.2 Add the `QuantizerDeltas` carrier and `dc_quantizer` / `ac_quantizer` implementing § 7.14.2 `get_dc_quant` / `get_ac_quant` per plane (luma AC delta is 0).
- [x] 2.3 Keep every function total and panic-free with `i64` clamp intermediates, reading no frame, segment, or tile state.
- [x] 2.4 Export the public items and update the crate and module documentation.

## 3. Tests

- [x] 3.1 Add `quantizer_index` tests for the three branches, the ignore-delta-q override, and both Clip3 bounds at the 8-bit and 10-bit `MaxQ`.
- [x] 3.2 Add per-plane `dc_quantizer` / `ac_quantizer` selection tests (including the luma AC-0 rule) and an end-to-end composition test reaching the qindex-0 special case.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency, concurrency, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-dequant-quantizer-index-resolution --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
