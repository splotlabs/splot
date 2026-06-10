# process delta: spec-coverage-doc

Advances `XTASK-FEATURE-STATUS`. Adds a generated, drift-gated per-spec-section
coverage view of the implementation matrix. No AV2 syntax change.

## ADDED Requirements

### Requirement: generated spec-coverage document

The repository SHALL provide a generated document `docs/SPEC-COVERAGE.md`,
rendered from `docs/IMPLEMENTATION-MATRIX.toml` by
`cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md`,
with one row per (spec section, feature) pair grouped by spec chapter and
ordered by a numeric-aware section key. Section cells SHALL hyperlink into the
committed spec mirror when the section resolves through
`docs/spec/av2/1.0.0/index.md` and SHALL fall back to plain text otherwise.
Features with no spec section SHALL be listed in a dedicated tail section.
`cargo xtask check-feature-status` SHALL fail when the committed document does
not match its render.

#### Scenario: looking up a spec section

- **WHEN** a reader opens `docs/SPEC-COVERAGE.md` and finds the row for a
  section such as § 5.4.4
- **THEN** the row shows the owning Feature ID and glyph statuses for mapped,
  parse, validate, and tests, plus a diagnostics count

#### Scenario: committed document drifts from the matrix

- **WHEN** `docs/IMPLEMENTATION-MATRIX.toml` changes without regenerating
  `docs/SPEC-COVERAGE.md`
- **THEN** `cargo xtask check-feature-status` (and therefore `cargo xtask ci`)
  fails and names the regenerate command

#### Scenario: spec mirror is absent or unresolvable

- **WHEN** a cited section cannot be resolved through the mirror index
- **THEN** the section renders as plain text and generation still succeeds
