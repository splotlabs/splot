# Change: validator-source-line-budget

## Feature IDs

- `XTASK-VALIDATOR-MODULE-SPLIT`
- `XTASK-SOURCE-LINES`

## Why

`crates/splot-validate/src/validator.rs` has grown into a 22k-line file, making
review and maintenance unnecessarily risky for a validator-first crate. The
repository also lacks an automated guardrail that warns when Rust source files
grow past a maintainability budget and fails when a new monster file appears.

## What Changes

- Split the validator module into `crates/splot-validate/src/validator/` with a
  small public facade, runner flow, diagnostic conversion helpers, and
  responsibility-oriented test modules.
- Preserve the public `splot_validate::Validator` API and all existing validator
  behavior, diagnostics, ordering, and tests.
- Document a soft Rust source-file size budget in `AGENTS.md`.
- Add `cargo xtask check-source-lines` and wire it into `cargo xtask ci`.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `process`: Adds the source-file size budget and its deterministic xtask
  report/check.

## Impact

- Crates/modules: `crates/splot-validate/src/validator/`,
  `crates/splot-validate/src/lib.rs`, and `xtask/src/main.rs`.
- Docs/tracking: `AGENTS.md`, `docs/IMPLEMENTATION-MATRIX.toml`,
  generated feature-status docs, and this OpenSpec change.
- Public API: no breaking changes; `splot_validate::Validator` remains exported
  through `pub use validator::Validator`.
- Dependencies: no new third-party dependencies.

## Non-goals

- No AV2 syntax, parser, validator semantic, diagnostic, or conformance behavior
  changes.
- No diagnostic `rule_id`, severity, spec section, location, message text, or
  ordering changes.
- No broad refactor of other large files merely because they exceed the soft
  budget today.
