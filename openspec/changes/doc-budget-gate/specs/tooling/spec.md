# tooling delta: doc-budget-gate

Adds a committed manual-documentation budget gate. Tracked by
`XTASK-DOC-BUDGET`.

## ADDED Requirements

### Requirement: committed manual markdown budget

The repository SHALL record the manual markdown budget in
`tools/docs/budget.toml`. The budget SHALL define counted manual markdown limits,
excluded paths, allowed manual docs, banned path patterns, and generated status
documents that must not be committed. Tracked by `XTASK-DOC-BUDGET`.

#### Scenario: counted manual docs are within budget

- **WHEN** `cargo xtask check-doc-budget` runs
- **THEN** it counts committed markdown not excluded by the budget
- **AND** it passes only when the counted file and line totals are within the
  configured limits

#### Scenario: generated status markdown is committed

- **WHEN** a generated status, coverage, or support markdown render is present
- **THEN** `cargo xtask check-doc-budget` fails and tells the developer to
  generate it on demand instead

### Requirement: documentation budget in acceptance gates

`cargo xtask ci` and the GitHub CI workflow SHALL run
`cargo xtask check-doc-budget`. Tracked by `XTASK-DOC-BUDGET`.

#### Scenario: CI runs repository gates

- **WHEN** the acceptance pipeline runs
- **THEN** documentation budget violations fail the pipeline before merge

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
