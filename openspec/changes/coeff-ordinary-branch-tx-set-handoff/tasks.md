## 1. Branch Handoff

- [x] 1.1 Add the crate-private `txSet` ordinary branch input/config types and
  typed domain errors.
- [x] 1.2 Derive AV2 §5.20.8.3 `txSet` from generated transform-size conversion
  tables and caller-resolved reduced-set facts before delegating to the existing
  `Mode_To_Txfm` ordinary branch.

## 2. Tests

- [x] 2.1 Add equivalence tests for default intra, reduced chroma, and DCT-only
  transform-set derivation.
- [x] 2.2 Add fail-atomic tests for invalid `reduced_tx_set` and transform-size
  table domains.

## 3. Tracking And Docs

- [x] 3.1 Add `DECODE-COEFF-ORDINARY-BRANCH-TX-SET-HANDOFF` to the
  implementation matrix with proof commands and residuals.
- [x] 3.2 Add decoder support/conformance coverage metadata and refresh
  generated status documents plus roadmap notes.
- [x] 3.3 Validate OpenSpec artifacts and feature-status consistency.

## 4. Verification

- [x] 4.1 Run focused coefficient-loop tests.
- [x] 4.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`,
  and `cargo xtask ci`.
