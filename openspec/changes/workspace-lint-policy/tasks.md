## 1. Planning And Feature Tracking

- [x] 1.1 Add OpenSpec intent for `XTASK-LINT-POLICY`.
- [x] 1.2 Add `XTASK-LINT-POLICY` to the implementation matrix and generated
  status docs.

## 2. Automation

- [x] 2.1 Add `cargo xtask check-lint-policy`.
- [x] 2.2 Wire the check into `cargo xtask ci`.
- [x] 2.3 Document the focused command in the agent command reference.

## 3. Tests And Validation

- [x] 3.1 Add focused `xtask` tests for approved allows, unknown allows,
  removed debt allows, required denies, and lint-group priority.
- [x] 3.2 Run focused `xtask` tests and the new lint-policy check.
- [x] 3.3 Run the relevant generated-doc and repository checks.
