# process delta: validator-context-split

Tracks `VALIDATOR-CONTEXT-SPLIT`. This is a maintainability-only refactor and
does not add AV2 conformance coverage.

## ADDED Requirements

### Requirement: validator context module organization

The private `splot-validate` validator context SHALL be organized as a real Rust
module tree under `crates/splot-validate/src/context/`, with
`crate::context::ValidatorContext` remaining available to crate-internal callers.
The split SHALL preserve validation behavior and diagnostic identity, including
rule IDs, severities, spec sections, offsets, messages, and ordering.

#### Scenario: crate-internal context path remains stable

- **WHEN** `splot-validate` is built after the split
- **THEN** existing crate-internal callers can still use
  `crate::context::ValidatorContext`

#### Scenario: source organization respects the file budget

- **WHEN** the context source tree is inspected
- **THEN** no production `context/*.rs` module exceeds the repository source-line
  hard cap or requires a special hard-cap allowance for the former monolithic
  `context.rs`

#### Scenario: diagnostics are preserved

- **WHEN** the existing validator test suite runs after the split
- **THEN** its diagnostic expectations remain unchanged
