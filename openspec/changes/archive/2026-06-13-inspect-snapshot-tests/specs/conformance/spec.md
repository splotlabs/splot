# conformance delta: inspect-snapshot-tests

Advances `CONF-INSPECT-SNAPSHOTS` by freezing the `splot inspect --json` output with golden
snapshots.

## ADDED Requirements

### Requirement: inspect output golden snapshots

The conformance suite SHALL include `insta` golden snapshot tests of the `splot inspect
--json` output over a diverse set of committed fixtures, so any change to the inspector's
per-OBU JSON for a committed fixture is surfaced as a reviewable snapshot diff. The
inspector output is deterministic (per-OBU byte offsets, sizes, and parsed fields, with no
paths, timestamps, or filenames), so the snapshots are stable across runs and machines.

#### Scenario: inspector output is frozen

- **WHEN** `splot inspect --json` is run against a committed fixture
- **THEN** its output matches the committed golden snapshot for that fixture

#### Scenario: an output change is surfaced

- **WHEN** the inspector's JSON output for a committed fixture changes
- **THEN** the snapshot test fails with a diff that must be explicitly reviewed and accepted

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
