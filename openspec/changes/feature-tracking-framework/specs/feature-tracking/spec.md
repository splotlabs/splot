# feature-tracking delta: feature-tracking-framework

## ADDED Requirements

### Requirement: canonical implementation matrix

The repository SHALL track AV2 implementation status in a canonical
`docs/IMPLEMENTATION-MATRIX.toml`, enforced by `cargo xtask check-feature-status`
and wired into `cargo xtask ci`.

#### Scenario: drift is rejected

- **WHEN** a `TODO(spec: <id>)`, a feature-id token, or a `done` stage without proof
  disagrees with the matrix
- **THEN** `cargo xtask check-feature-status` fails with an actionable message

### Requirement: rendered status

The matrix SHALL be renderable as a table, JSON, and markdown, and summarizable by
`cargo xtask spec-coverage`.

#### Scenario: regenerate the status doc

- **WHEN** `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md` runs
- **THEN** `docs/FEATURE-STATUS.md` reflects the matrix and the drift check passes
