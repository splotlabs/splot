## 1. Planning And Feature Tracking

- [x] 1.1 Create the OpenSpec change with proposal, design, specs, and tasks.
- [x] 1.2 Add `RECON-PC-WIENER-CLASSIFICATION` to the implementation matrix,
  decoder support matrix, and generated status/coverage documents.

## 2. Shared Tables

- [x] 2.1 Teach `cargo xtask gen-tables` to emit the generated AV2 §9.8
  loop-restoration table module into `splot-tables` while preserving the existing
  `splot-core` module.
- [x] 2.2 Regenerate tables and add a focused `splot-tables` spot test for the
  shared loop-restoration table exposure.

## 3. Reconstruction Implementation

- [x] 3.1 Add a new `splot-recon` PC-Wiener classification module with public
  parameter/result types and `pc_wiener_classify`.
- [x] 3.2 Implement AV2 §7.20.4 feature accumulation, normalization,
  `get_qval_given_tskip`, `lutInput` construction, and normative LUT lookup over
  caller-resolved source samples and `LrTxSkip` values.
- [x] 3.3 Export the primitive and update crate docs without runtime decode
  rewiring.

## 4. Tests

- [x] 4.1 Add focused `splot-recon` tests for flat-source classification,
  hand-computed features, `LrTxSkip` quantizer contribution, and 8-bit/10-bit
  normalization.
- [x] 4.2 Add negative tests for unsupported sample storage, source samples
  outside the active bit-depth range, and invalid `LrTxSkip` values.

## 5. Validation And PR Discipline

- [x] 5.1 Run `openspec validate recon-pc-wiener-classification --strict`.
- [x] 5.2 Run focused tests, `cargo xtask gen-tables --check`, feature-status,
  decoder-support, decoder conformance coverage, and dependency-direction gates.
- [x] 5.3 Re-run local decoder mission hash decode to verify the frontier remains structured and
  honest until runtime value wiring exists.
- [x] 5.4 If all tasks complete and a PR is ready, sync/archive this OpenSpec
  change before merge, request Claude and Codex reviews, wait for both latest-head
  responses, and address actionable feedback before merging.
