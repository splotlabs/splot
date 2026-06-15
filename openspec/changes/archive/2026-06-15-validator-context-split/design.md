# Design: validator-context-split

## Context

`crates/splot-validate/src/context.rs` had become a monolithic validator state
machine. The change is a maintainability refactor only: it preserves validator
behavior while making the context easier to audit and keeping the former
monolithic source file out of source-line allowance pressure.

## Data model / API

The crate-internal API remains `crate::context::ValidatorContext`. The
implementation lives under `crates/splot-validate/src/context/` and splits helper
state and method groups by validation responsibility. The module boundary stays
private to `splot-validate`; no public API, crate dependency, diagnostic, or CLI
surface changes.

## Spec mapping

This change does not add AV2 syntax or conformance coverage. It is tracked by
`VALIDATOR-CONTEXT-SPLIT` as infrastructure, with no `spec_sections` in
`docs/IMPLEMENTATION-MATRIX.toml`.

## Diagnostics

Diagnostic identity is intentionally unchanged. Existing validator rule IDs,
severities, spec sections, byte/bit offsets, messages, and ordering remain
governed by the moved validator code and existing tests.

## Tests

The completed change is proven by the existing validator and repository gates:

- `cargo check -p splot-validate --all-targets --locked`
- `cargo test -p splot-validate --all-targets --locked`
- `cargo test -p xtask --all-targets --locked`
- `cargo xtask feature-status`
- `cargo xtask check-feature-status`
- `cargo xtask ci`

## Alternatives considered

- Leave `context.rs` monolithic: rejected because the file was difficult to
  review and audit and kept pressure on the source-line budget.
- Split into public submodules: rejected because callers do not need a broader
  API surface.

## Risks

- Spec ambiguity: none; this is not AV2 semantic work.
- Performance: negligible; this is a source organization change.
- Compatibility: low risk because `crate::context::ValidatorContext` remains the
  stable crate-internal entry point.
- Maintenance: improved by grouping validation responsibilities into cohesive
  modules.
