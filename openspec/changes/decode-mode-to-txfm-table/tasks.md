## 1. Generator

- [x] 1.1 Add a narrow AV2 `TxType` symbol resolver for `Mode_To_Txfm`.
- [x] 1.2 Remove `Mode_To_Txfm` from the generator skip allowlist and regenerate
  generated table outputs.

## 2. Tests

- [x] 2.1 Add generator unit tests for supported and rejected `TxType` symbol
  resolution.
- [x] 2.2 Add core table spot checks proving `MODE_TO_TXFM` against the AV2 §9.2
  mirror and §3 symbol values.

## 3. Tracking And Docs

- [x] 3.1 Add `DECODE-MODE-TO-TXFM-SYMBOLIC-TABLE` to the implementation matrix
  with proof commands and residuals.
- [x] 3.2 Add decoder support/conformance coverage metadata and refresh generated
  status documents plus roadmap notes.
- [x] 3.3 Validate OpenSpec artifacts and feature-status consistency.

## 4. Verification

- [x] 4.1 Run targeted generator/table tests and `cargo xtask gen-tables --check`.
- [x] 4.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`,
  and `cargo xtask ci`.
