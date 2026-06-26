# tooling delta: dupehound-duplication-gate

Adds a duplicate-code budget gate to the `tooling` capability, sibling to the
zero-copy and concurrency runtime policies. Non-normative repository tooling: it
adds no AV2 conformance coverage. Tracked by `INFRA-DUPEHOUND-DUPLICATION-GATE`.

## ADDED Requirements

### Requirement: committed absolute duplicate-code budget

The repository SHALL record an absolute duplicate-code ceiling in
`tools/dupehound/budget.toml` as `max_deletable_lines`. The ceiling is a ratchet:
it SHALL be lowered when a duplicate cluster is removed and SHALL NOT be raised.
Tracked by `INFRA-DUPEHOUND-DUPLICATION-GATE`.

#### Scenario: budget records the ceiling

- **WHEN** the duplicate-code gate runs
- **THEN** it reads `max_deletable_lines` from `tools/dupehound/budget.toml` as the
  ceiling to enforce

### Requirement: enforcement by check-duplication

The duplicate-code budget SHALL be enforced by `cargo xtask check-duplication`,
which runs `dupehound scan --include-tests --json`, compares
`score.deletable_lines` against the committed ceiling, and runs in `cargo xtask
ci` alongside the other repository gates. It SHALL follow the run-if-present
policy: mandatory in CI (the workflow installs `dupehound`) and skipped with an
install hint when the binary is absent. Tracked by
`INFRA-DUPEHOUND-DUPLICATION-GATE`.

#### Scenario: measured duplication exceeds the budget

- **WHEN** `dupehound scan --include-tests` reports more deletable duplicate lines
  than `max_deletable_lines`
- **THEN** `cargo xtask check-duplication` fails with the offending count and the
  ceiling, and the build does not pass

#### Scenario: measured duplication is within the budget

- **WHEN** the reported deletable-line count is at or below the ceiling
- **THEN** the gate passes, and when under budget it reports the headroom and the
  lower value to ratchet the budget to

#### Scenario: dupehound is not installed locally

- **WHEN** `cargo xtask check-duplication` runs on a checkout without the
  `dupehound` binary
- **THEN** it prints an install hint and returns success, so a fresh checkout can
  still run `cargo xtask ci`

### Requirement: per-PR newly-introduced-duplication ratchet

Continuous integration SHALL run `dupehound check --diff <base>` on pull requests
to fail when the PR's diff duplicates existing code, blocking newly introduced
duplication independently of the absolute budget. The base revision SHALL be bound
through an environment variable rather than interpolated into the command.
Tracked by `INFRA-DUPEHOUND-DUPLICATION-GATE`.

#### Scenario: a pull request introduces duplication

- **WHEN** a pull request adds a function that duplicates existing code
- **THEN** the CI `dupehound check --diff <base>` step fails and identifies the
  original to reuse

#### Scenario: a push event has no pull-request base

- **WHEN** the workflow runs on a push event rather than a pull request
- **THEN** the `check --diff` step is skipped while the absolute budget gate still
  runs

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
