# validator delta: validate-explain-rule

Tracks `CLI-VALIDATE-EXPLAIN`. This adds a read-only CLI catalog over the existing
validator diagnostics; it changes no validator behavior and emits no diagnostics.

## ADDED Requirements

### Requirement: explain diagnostic registry

The toolkit SHALL provide a diagnostic registry, generated from
`docs/VALIDATOR-DIAGNOSTICS.md` by `cargo xtask gen-explain`, mapping each emitted
validator rule id to its severity, AV2 spec section, and one-line summary. The
registry SHALL be drift-checked: `cargo xtask gen-explain --check`, run by
`cargo xtask ci`, SHALL fail if the generated table diverges from the doc. The
registry SHALL NOT contain hand-authored or invented data.

#### Scenario: registry stays in sync with the doc

- **WHEN** the generated registry matches the doc's emitted-diagnostics tables
- **THEN** `cargo xtask gen-explain --check` exits zero

#### Scenario: registry drift is rejected

- **WHEN** the doc's diagnostics tables change without regenerating the table
- **THEN** `cargo xtask gen-explain --check` exits non-zero

### Requirement: explain command

`splot explain <rule-id>` SHALL describe a known diagnostic — its rule id,
severity, spec section, and summary — in human text or, with `--json`, as a
machine-readable object. `splot explain --list` SHALL enumerate every known rule id
(text: one id per line, sorted; `--json`: the full catalog). An unknown rule id or a
missing argument SHALL produce a clean error on stderr with a non-zero exit code and
SHALL NOT panic; an unknown id SHALL include a same-namespace "did you mean" hint.

#### Scenario: describe a known rule id

- **WHEN** `splot explain <known-rule-id>` runs
- **THEN** it prints the id, severity, spec section, and summary and exits zero

#### Scenario: unknown rule id

- **WHEN** `splot explain <unknown-rule-id>` runs
- **THEN** it prints an "unknown rule id" error with a suggestion and exits non-zero
  (no panic)

#### Scenario: list all rule ids

- **WHEN** `splot explain --list` runs
- **THEN** it prints every known rule id, sorted, and exits zero
