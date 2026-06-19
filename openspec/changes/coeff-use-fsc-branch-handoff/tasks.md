## 1. Selector Implementation

- [x] 1.1 Add a focused `coeff_loop/use_fsc_branch.rs` module with crate-private input, result, and error types for `DECODE-COEFF-USE-FSC-BRANCH-HANDOFF`.
- [x] 1.2 Implement all-zero routing through the ordinary all-zero branch without evaluating `use_fsc` or FSC-specific facts.
- [x] 1.3 Implement nonzero `use_fsc == false` delegation through `apply_coeff_ordinary_branch_from_lossless`.
- [x] 1.4 Implement nonzero `use_fsc == true` delegation through `apply_coeff_fsc_branch_from_tx_size`.
- [x] 1.5 Wire the new module into the coefficient-loop module tree without changing public APIs or crate dependencies.

## 2. Tests

- [x] 2.1 Add focused tests proving all-zero selector output matches direct ordinary all-zero behavior.
- [x] 2.2 Add focused tests proving nonzero ordinary selector output matches direct ordinary lower-boundary behavior.
- [x] 2.3 Add focused tests proving nonzero FSC selector output matches direct FSC tx-size lower-boundary behavior.
- [x] 2.4 Add negative tests proving selected-branch errors are typed and preserve state/CDF/symbol position according to lower-branch preflight guarantees.
- [x] 2.5 Run focused `splot-decode` coefficient-loop tests.

## 3. Tracking and Documentation

- [x] 3.1 Add `DECODE-COEFF-USE-FSC-BRANCH-HANDOFF` to `docs/IMPLEMENTATION-MATRIX.toml` with proof tests and commands.
- [x] 3.2 Add the decoder-support row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 3.3 Update `docs/DECODER-ROADMAP.md` and decoder conformance coverage metadata for the new partial handoff.
- [x] 3.4 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Validation

- [x] 4.1 Run `openspec validate coeff-use-fsc-branch-handoff --strict`.
- [x] 4.2 Run `openspec validate --all --no-interactive`.
- [x] 4.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.4 Run `git diff --check`.
- [x] 4.5 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
