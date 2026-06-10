# feature-tracking delta: matrix-validate-stage-honesty-sweep

Formalizes the roadmap done-criteria rule the sweep enforces: a partial stage
must say what remains. No AV2 syntax change.

## ADDED Requirements

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

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
