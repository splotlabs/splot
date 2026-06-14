# validator delta: docs-validator-examples

Tracks `DOC-VALIDATOR-EXAMPLES`. This is a documentation-only change; it adds no
behavior and emits no diagnostics. It records the expectation that the validator's
user-facing CLI surface is documented with worked, non-invented examples.

## ADDED Requirements

### Requirement: documented validator CLI surface

The README SHALL document each user-facing validator/inspector subcommand and flag
— at minimum `splot validate` (including `--json`, `--strict`, `--max-diagnostics`,
and `--summary-only`), `splot inspect`, and `splot explain` (including `--json` and
`--list`) — with a worked example. Every example's shown output SHALL match the
output of the shipped `splot` binary; the README SHALL NOT show invented or
aspirational output for a command or flag that does not behave that way.

#### Scenario: new user-facing surface is documented

- **WHEN** a user-facing validator/inspector subcommand or flag ships
- **THEN** the README documents it with an example whose output matches the binary

#### Scenario: examples stay truthful

- **WHEN** a documented command is run as shown in the README
- **THEN** its real output matches the example (no invented behavior or output)
