## 1. Branch Handoff

- [x] 1.1 Add crate-private lossless ordinary branch input/config types above
  the existing `txSet` handoff.
- [x] 1.2 Implement the AV2 §5.20.7.29 lossless `DCT_DCT` short-circuit before
  lower `txSet`/`Mode_To_Txfm` validation, while delegating non-lossless paths
  to the existing `txSet` wrapper.

## 2. Tests

- [x] 2.1 Add equivalence tests for the lossless `DCT_DCT` short-circuit and
  non-lossless `txSet` delegation.
- [x] 2.2 Add fail-atomic tests for invalid transform-size domains and coverage
  proving lossless bypasses lower non-lossless validation.

## 3. Tracking And Docs

- [x] 3.1 Add `DECODE-COEFF-ORDINARY-BRANCH-LOSSLESS-HANDOFF` to the
  implementation matrix with proof commands and residuals.
- [x] 3.2 Add decoder support/conformance coverage metadata and refresh
  generated status documents plus roadmap notes.
- [x] 3.3 Validate OpenSpec artifacts and feature-status consistency.

## 4. Verification

- [x] 4.1 Run focused coefficient-loop tests.
- [x] 4.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`,
  and `cargo xtask ci`.
