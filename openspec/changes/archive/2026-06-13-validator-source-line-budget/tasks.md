# Tasks

## 1. Tracking and Docs

- [x] 1.1 Add `XTASK-VALIDATOR-MODULE-SPLIT` and `XTASK-SOURCE-LINES` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Document the Rust source-file size budget in `AGENTS.md`.
- [x] 1.3 Regenerate `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`.

## 2. Validator Module Split

- [x] 2.1 Replace `crates/splot-validate/src/validator.rs` with a `validator/` module tree.
- [x] 2.2 Move public `Validator` API into `validator/mod.rs` without changing `splot_validate::Validator`.
- [x] 2.3 Move stream orchestration/check execution into `validator/runner.rs`.
- [x] 2.4 Move parse/IVF diagnostic conversion helpers into `validator/diagnostics.rs`.
- [x] 2.5 Split validator tests into responsibility-oriented files under `validator/tests/`.
- [x] 2.6 Confirm no validator test was deleted and no new validator module file remains near the old monster size.

## 3. Source-Line Xtask

- [x] 3.1 Add `cargo xtask check-source-lines` command dispatch/help.
- [x] 3.2 Implement deterministic Rust source line counting with a 1000-line soft limit and hard-cap failure.
- [x] 3.3 Wire `check-source-lines` into `cargo xtask ci`.
- [x] 3.4 Add or update xtask tests for soft warning, hard failure, and hard-cap exceptions.

## 4. Verification

- [x] 4.1 Run `cargo fmt --all`.
- [x] 4.2 Run `cargo test -p splot-validate --all-targets --locked`.
- [x] 4.3 Run `cargo xtask check-source-lines`.
- [x] 4.4 Run `cargo xtask check-diagnostic-registry`.
- [x] 4.5 Run `cargo xtask check-feature-status`.
- [x] 4.6 Run `cargo xtask ci`.
