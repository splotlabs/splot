# process delta: validator-source-line-budget

Advances `XTASK-VALIDATOR-MODULE-SPLIT` and `XTASK-SOURCE-LINES`. Adds a
repository process guarantee for a reviewable Rust source-file size budget and
deterministic line-count check. No AV2 syntax, parser, validator semantic, or
diagnostic behavior changes.

## ADDED Requirements

### Requirement: Rust source-file size budget

The repository SHALL document a soft Rust source-file budget of 1000 physical
lines and provide `cargo xtask check-source-lines` to inspect tracked and
non-ignored new `.rs` files offline. The check SHALL print advisory warnings for
files above the soft budget and SHALL fail when a non-exempt Rust file exceeds
the configured hard cap. `cargo xtask ci` SHALL run the check.

#### Scenario: Rust file exceeds the soft budget only

- **WHEN** a checked Rust source file has more than 1000 physical lines but does
  not exceed the hard cap
- **THEN** `cargo xtask check-source-lines` prints an advisory warning and exits
  successfully

#### Scenario: Rust file exceeds the hard cap

- **WHEN** a checked Rust source file exceeds the hard cap and has no documented
  exception
- **THEN** `cargo xtask check-source-lines` fails and names the offending file

#### Scenario: source-line check runs in CI

- **WHEN** `cargo xtask ci` runs
- **THEN** it also runs `cargo xtask check-source-lines` as a deterministic
  repository check

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
