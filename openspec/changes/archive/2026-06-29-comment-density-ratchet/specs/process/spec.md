## ADDED Requirements

### Requirement: implementation-comment density ratchet

The repository SHALL enforce a ratcheting budget for full-line implementation
comments in Rust source under `crates`, `xtask`, and `fuzz/fuzz_targets`,
tracked by `INFRA-COMMENT-DENSITY-RATCHET`. The gate SHALL exclude SPDX license
lines, Rustdoc, and xtask-generated Rust sources, SHALL count untracked
non-ignored Rust files during local runs, and SHALL run in both `cargo xtask ci`
and GitHub CI. The budget SHALL live in `tools/comments/budget.toml` and SHALL
only be raised with maintainer approval.

#### Scenario: source comments stay within budget

- **WHEN** `cargo xtask check-comment-density` runs with full-line
  implementation comments at or below the configured budget
- **THEN** the command succeeds and reports the current count

#### Scenario: source comments exceed budget

- **WHEN** `cargo xtask check-comment-density` runs with full-line
  implementation comments above the configured budget
- **THEN** the command fails and reports the current count, budget, and overage
