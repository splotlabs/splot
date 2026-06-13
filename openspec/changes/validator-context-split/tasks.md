# Tasks

## Tracking

- [x] Add `VALIDATOR-CONTEXT-SPLIT` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Keep the OpenSpec change scoped to refactor-only maintainability.

## Implementation

- [x] Move `crates/splot-validate/src/context.rs` to `context/mod.rs`.
- [x] Extract helper state and `ValidatorContext` method groups by responsibility.
- [x] Keep `crate::context::ValidatorContext` and the crate-internal call sites compiling.
- [x] Preserve existing tests without weakening expectations.
- [x] Remove any old source-line hard-cap allowance if present.

## Checks

- [x] `cargo fmt --all`
- [x] `cargo check -p splot-validate --all-targets --locked`
- [x] `cargo test -p splot-validate --all-targets --locked`
- [x] `cargo test -p xtask --all-targets --locked`
- [x] `cargo xtask feature-status`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
