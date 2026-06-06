# process Specification

## Purpose

Repository process guardrails for source provenance, review hygiene, and CI-enforced
contribution requirements.

Tracked by Feature IDs: `DOC-ENCODER-REFERENCE-GATE`,
`XTASK-CONVENTIONAL-COMMITS`.

## Requirements

### Requirement: encoder reference gate

The repository SHALL require contributors to consult the reference notes before
encoder work uses rav1e, SVT-AV1, or another AV1 implementation as research input,
and to confirm the change does not copy AV1 syntax, constants, tables, comments,
prose, or decoder-visible semantics into `splot`. Tracked by
`DOC-ENCODER-REFERENCE-GATE`.

#### Scenario: encoder change uses an AV1 implementation as research input

- **WHEN** a contributor opens an encoder-facing change informed by rav1e, SVT-AV1,
  or another AV1 implementation
- **THEN** the PR explains the source as research context only and records any
  decoder-visible AV2 mapping work separately

### Requirement: conventional PR titles and commit subjects

Repository pull request titles and commit subjects SHALL use Conventional Commits
text with the format `<type>[optional scope][!]: <description>`, enforced by
`cargo xtask check-conventional-title`, `cargo xtask check-conventional-commits`,
and CI. Tracked by `XTASK-CONVENTIONAL-COMMITS`.

#### Scenario: non-conventional pull request title

- **WHEN** a pull request title does not match the documented Conventional Commits
  format
- **THEN** the CI title check fails with the offending title and the allowed type
  list

#### Scenario: non-conventional commit subject

- **WHEN** a pull request or push contains a commit subject that does not match the
  documented Conventional Commits format
- **THEN** the CI commit-message check fails with the offending commit subject and
  the allowed type list
