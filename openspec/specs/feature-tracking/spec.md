# feature-tracking Specification

## Purpose

The repository's canonical AV2 implementation-status model, drift checks, and
on-demand rendered status outputs.

Tracked by Feature ID: `XTASK-FEATURE-STATUS`.

## Requirements

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
`cargo xtask spec-coverage`. Generated markdown outputs SHALL be on-demand
artifacts: they may be produced locally, and if committed they SHALL be
drift-checked, but absence is allowed.

#### Scenario: regenerate the status doc

- **WHEN** `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md` runs
- **THEN** `docs/FEATURE-STATUS.md` reflects the matrix

### Requirement: partial stages name their residual or blocker

A normative matrix row SHALL NOT carry a bare `partial` validate stage: its
`notes` field SHALL state either the concrete remaining locally-decidable work,
the matrix row that owns the residual semantics, or the blocking dependency
(a named parsing feature, decoder process, or external input) that prevents
closing the stage.

#### Scenario: residual owned by another row

- **WHEN** a row's remaining validate semantics are tracked by a different
  matrix row
- **THEN** the row's notes name that owner row id and the stage reflects only
  the row's own scope

#### Scenario: decoder-blocked residual

- **WHEN** a check cannot be implemented without a decoding process the
  validator intentionally does not have
- **THEN** the row's notes say so explicitly (naming the blocking spec
  process) and the stage stays `partial` as documented-blocked rather than
  silently unfinished

#### Scenario: parse-blocked residual

- **WHEN** a check is decidable only after parsing work that another matrix
  row or backlog change owns (for example frame-header inter-path parsing)
- **THEN** the row's notes name that blocking feature id and the stage stays
  `partial` as documented-blocked until the parsing lands
