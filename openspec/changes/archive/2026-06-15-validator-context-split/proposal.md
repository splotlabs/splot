# Change: validator-context-split

## Feature IDs

- `VALIDATOR-CONTEXT-SPLIT`

## Why

`crates/splot-validate/src/context.rs` has grown into a single large validator
state machine. The behavior is valuable, but the file is difficult to review,
audit, and keep under the repository source-file budget.

## Scope

- Spec sections: none newly modeled.
- Crates/modules: `crates/splot-validate/src/context/`.
- CLI/docs/tests: no public CLI or diagnostic behavior changes; move existing
  context unit tests with their domain code where practical.

## Non-goals

- Do not add AV2 conformance coverage.
- Do not change diagnostic rule IDs, severities, spec sections, messages,
  byte/bit offsets, or ordering.
- Do not add dependencies, unsafe code, generated includes, or source-line
  budget exceptions.

## Acceptance criteria

- [ ] `ValidatorContext` remains available as `crate::context::ValidatorContext`.
- [ ] The private `context` module is split into cohesive Rust modules.
- [ ] Existing validation behavior and diagnostics are preserved.
- [ ] Existing tests are preserved.
- [ ] All new Rust files have SPDX headers.
- [ ] `cargo xtask check-feature-status` passes.
- [ ] `cargo xtask ci` passes or any local blockage is documented.
