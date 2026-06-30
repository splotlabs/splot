## ADDED Requirements

### Requirement: Generic runtime cleanup

The decoder runtime SHALL replace repeated local runtime plumbing with shared
crate-private helpers while preserving existing decoded output and structured
diagnostics.

#### Scenario: Runtime behavior stays stable

- **WHEN** the cleanup is applied
- **THEN** focused decoder tests and `cargo xtask ci` pass without changing
  existing unsupported-feature rule IDs.

#### Scenario: Budget gates preserve cleanup gains

- **WHEN** repository cleanup gates run
- **THEN** comment density is at the tracked 262 budget, duplication stays at or
  below the tracked 6327 budget, and source-line hard allowances remain empty.
