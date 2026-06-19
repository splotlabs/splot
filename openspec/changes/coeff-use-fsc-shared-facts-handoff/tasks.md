## 1. Shared-Facts Wrapper

- [x] 1.1 Add crate-private shared-facts input types for `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF`.
- [x] 1.2 Implement all-zero routing through the existing ordinary all-zero selector path without requiring nonzero facts.
- [x] 1.3 Implement nonzero `useFsc` derivation from shared `enable_fsc`, `PlaneTxType`, `plane`, `fsc_mode`, and `is_inter` facts.
- [x] 1.4 Lazily construct only the selected ordinary or FSC lower branch input and delegate without changing public APIs or crate dependencies.

## 2. Tests

- [x] 2.1 Add focused tests proving all-zero shared-facts output matches the lower selector all-zero path.
- [x] 2.2 Add focused tests proving derived false conditions match the lower ordinary branch path.
- [x] 2.3 Add focused tests proving derived true conditions match the lower FSC branch path.
- [x] 2.4 Add selected-branch-only validation coverage proving invalid non-selected facts are ignored.
- [x] 2.5 Run focused `splot-decode` coefficient-loop tests.

## 3. Tracking and Documentation

- [x] 3.1 Add `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF` to `docs/IMPLEMENTATION-MATRIX.toml` with proof tests and commands.
- [x] 3.2 Add the decoder-support row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 3.3 Update `docs/DECODER-ROADMAP.md` and decoder conformance coverage metadata for the new partial handoff.
- [x] 3.4 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Validation

- [x] 4.1 Run `openspec validate coeff-use-fsc-shared-facts-handoff --strict`.
- [x] 4.2 Run `openspec validate --all --no-interactive`.
- [x] 4.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.4 Run `git diff --check`.
- [x] 4.5 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
