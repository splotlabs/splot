## 1. Branch Handoff

- [x] 1.1 Add the crate-private `Mode_To_Txfm` ordinary branch input/config
  types and typed errors.
- [x] 1.2 Derive non-lossless intra chroma non-directional `PlaneTxType` from
  generated `MODE_TO_TXFM` plus the inline `Tx_Type_In_Set_Intra` table before
  delegating to the existing `PlaneTxType` ordinary branch.

## 2. Tests

- [x] 2.1 Add equivalence tests for accepted `Mode_To_Txfm` mappings and
  fallback-to-`DCT_DCT` behavior.
- [x] 2.2 Add fail-atomic tests for unsupported/out-of-domain subset inputs.

## 3. Tracking And Docs

- [x] 3.1 Add `DECODE-COEFF-ORDINARY-BRANCH-MODE-TO-TXFM-HANDOFF` to the
  implementation matrix with proof commands and residuals.
- [x] 3.2 Add decoder support/conformance coverage metadata and refresh
  generated status documents plus roadmap notes.
- [x] 3.3 Validate OpenSpec artifacts and feature-status consistency.

## 4. Verification

- [x] 4.1 Run focused coefficient-loop tests.
- [x] 4.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`,
  and `cargo xtask ci`.
