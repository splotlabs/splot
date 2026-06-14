# conformance delta: cli-snapshot-coverage

Tracks `CONF-CLI-SNAPSHOT-COVERAGE`. This extends the inspector/CLI snapshot test
layer; it does not add AV2 conformance coverage or change any CLI behavior.

## ADDED Requirements

### Requirement: CLI help-surface snapshots

The test suite SHALL freeze the `splot validate --help` and `splot inspect --help`
output as committed `insta` golden snapshots, so any change to those subcommands'
argument surface (a new, renamed, removed, or reordered flag, or a changed help
string) is surfaced as a reviewable snapshot diff. The snapshots SHALL be
deterministic — no filesystem paths, timestamps, or version strings — and the
top-level `splot --help` SHALL NOT be snapshotted.

#### Scenario: help surface unchanged

- **WHEN** the committed goldens match the current `validate`/`inspect` `--help`
- **THEN** the snapshot tests pass with no pending snapshots

#### Scenario: a flag is added or renamed

- **WHEN** a `validate` or `inspect` flag is added, renamed, or removed
- **THEN** the corresponding help snapshot diffs, requiring an explicit golden
  update in the same change

### Requirement: inspector text-output snapshots

The test suite SHALL freeze the `splot inspect` human (text) output — both the
default per-OBU dump and the `--headers` header-only dump — as committed `insta`
golden snapshots over representative committed fixtures, complementing the existing
`--json` snapshots. The text output is deterministic for a fixed input.

#### Scenario: text dump is stable

- **WHEN** `splot inspect` (default or `--headers`) runs against a committed fixture
- **THEN** its stdout matches the committed golden exactly
