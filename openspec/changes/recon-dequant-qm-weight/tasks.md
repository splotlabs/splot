## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-DEQUANT-QM-WEIGHT` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Implementation

- [x] 2.1 Relocate the generated § 9.4 `quantizer` module to `splot-tables` via `output_dir_for`; regenerate and confirm byte-identical (drift check green).
- [x] 2.2 Add `quantization_matrix_weight(QmWeightIndex)` (bounds-checked built-in `Quantizer_Matrix` lookup) and `qm_weighted_quantizer(q, m)` (`Round2(q*m, 5)`, total) to `dequant_process.rs`; add the typed `InvalidQuantizerMatrixIndex` error.
- [x] 2.3 Update the shared-tables enumeration in the implementation matrix, `AGENTS.md`, and `docs/ARCHITECTURE.md` (§ 9.4 now lives in `splot-tables`).

## 3. Tests

- [x] 3.1 Add the `Round2(q*m, 5)` vector and totality extreme for `qm_weighted_quantizer`.
- [x] 3.2 Add the built-in `Quantizer_Matrix` lookup match (luma/chroma, offset position) and the out-of-range rejection for `quantization_matrix_weight`.
- [x] 3.3 Run focused `splot-recon` / `splot-tables` tests plus clippy, gen-tables drift, dependency, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-dequant-qm-weight --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
