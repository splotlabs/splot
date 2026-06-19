## 1. Condition Wrapper

- [x] 1.1 Add crate-private condition input types for `DECODE-COEFF-USE-FSC-CONDITION-HANDOFF`.
- [x] 1.2 Implement all-zero routing through the existing selector without requiring condition facts.
- [x] 1.3 Implement nonzero `useFsc` derivation from `enable_fsc`, `PlaneTxType`, `plane`, `fsc_mode`, and `is_inter`.
- [x] 1.4 Delegate derived nonzero inputs into `apply_coeff_use_fsc_branch` without changing public APIs or crate dependencies.

## 2. Tests

- [x] 2.1 Add focused tests proving all-zero condition-wrapper output matches the lower selector all-zero path.
- [x] 2.2 Add focused tests proving derived false conditions delegate to the lower explicit ordinary selector path.
- [x] 2.3 Add focused tests proving derived true conditions delegate to the lower explicit FSC selector path.
- [x] 2.4 Add negative/contradictory-fact coverage proving non-selected branch facts are ignored.
- [x] 2.5 Run focused `splot-decode` coefficient-loop tests.

## 3. Tracking and Documentation

- [x] 3.1 Add `DECODE-COEFF-USE-FSC-CONDITION-HANDOFF` to `docs/IMPLEMENTATION-MATRIX.toml` with proof tests and commands.
- [x] 3.2 Add the decoder-support row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 3.3 Update `docs/DECODER-ROADMAP.md` and decoder conformance coverage metadata for the new partial handoff.
- [x] 3.4 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Validation

- [x] 4.1 Run `openspec validate coeff-use-fsc-condition-handoff --strict`.
- [x] 4.2 Run `openspec validate --all --no-interactive`.
- [x] 4.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.4 Run `git diff --check`.
- [x] 4.5 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
