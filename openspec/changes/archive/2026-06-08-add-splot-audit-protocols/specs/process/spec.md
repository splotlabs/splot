## ADDED Requirements

### Requirement: documentation audit protocol

The repository SHALL provide a documentation audit protocol that checks
project-authored agent guidance and documentation for stale claims, broken paths,
duplicated rules, contradictions, size drift, and misplaced guidance. The protocol
SHALL treat the AV2 specification mirror as read-only third-party material and
SHALL NOT edit production Rust code.

#### Scenario: scheduled documentation audit finds stale guidance

- **WHEN** the documentation audit runs and finds stale or broken project-authored
  guidance
- **THEN** the audit output proposes documentation-only fixes or recommendations
  for human review

#### Scenario: documentation audit reaches the spec mirror

- **WHEN** the documentation audit encounters files under `docs/spec/av2/<version>/`
- **THEN** it treats those files as read-only evidence and does not propose
  hand-edits to them

### Requirement: cross-agent audit skill exposure

The repository SHALL expose audit protocols through the project skill directories
recognized by the supported agent surfaces used by the repository. Claude Code
project skills SHALL live under `.claude/skills/`, Codex project skills SHALL live
under `.codex/skills/`, and any GitHub-hosted skill or prompt mirror SHALL live
under the existing `.github/skills/` or `.github/prompts/` assistant-integration
paths. The repository SHALL NOT rely on `.agents/skills/` as the only project skill
location.

#### Scenario: Claude Code needs the documentation audit

- **WHEN** Claude Code is started in the repository and a user asks for a `splot`
  documentation audit
- **THEN** the documentation audit skill is available from `.claude/skills/`

#### Scenario: Codex needs the AV2 conformance audit

- **WHEN** Codex is started in the repository and a user asks for a heavy `splot`
  AV2 conformance audit
- **THEN** the AV2 conformance audit skill is available from `.codex/skills/`

### Requirement: AV2 conformance audit protocol

The repository SHALL provide a heavy AV2 conformance audit protocol that reviews
changed implementation, validator, documentation, matrix, and assistant-integration
files against the committed AV2 spec mirror, `docs/SPEC-MAPPING.md`,
`docs/IMPLEMENTATION-MATRIX.toml`, and the repository rules. The protocol SHALL
separate audit findings from implementation fixes and SHALL require human review
for ambiguous AV2 spec interpretation. The protocol SHALL cover current and future
codec-facing areas, including parser, validator, encoder, decoder, writer,
inspector, conformance, fuzzing, and automation work as those areas are added to
the workspace.

#### Scenario: changed parser file is selected for audit

- **WHEN** a changed file under `crates/splot-core` parses AV2 syntax
- **THEN** the audit checks that syntax claims cite resolvable AV2 sections or
  known `TODO(spec: <FEATURE-ID>)` markers, and reports unsupported or invented
  behavior as findings

#### Scenario: changed validator file is selected for audit

- **WHEN** a changed file under `crates/splot-validate` emits or modifies
  diagnostics
- **THEN** the audit checks stable `rule_id`, severity, spec section, offset
  handling, matrix proof, and tests for the affected behavior

#### Scenario: future encoder or decoder file is selected for audit

- **WHEN** a changed file belongs to an encoder, decoder, writer, inspector, or
  other codec-facing workspace member added after this protocol
- **THEN** the audit applies the same AV2 spec-grounding, matrix, tests, safety,
  and provenance checks instead of skipping the file because its crate was unknown

#### Scenario: ambiguous spec interpretation

- **WHEN** the audit cannot determine whether an implementation claim matches the
  AV2 specification
- **THEN** it records a human-required finding instead of guessing or modifying
  behavior

### Requirement: deterministic audit scope

The repository SHALL provide deterministic tooling for computing the candidate
files and impacted Feature IDs for the heavy AV2 conformance audit. The tooling
SHALL support both PR/diff-based operation and scheduled default-branch operation
using a persisted content-hash ledger. The tooling SHALL discover workspace
members and in-scope repository paths dynamically rather than hardcoding only the
crates and docs present when the tool was first written.

#### Scenario: unchanged file is skipped by scheduled audit

- **GIVEN** the audit ledger records a successful audit for a file content hash
- **WHEN** a scheduled audit computes scope and the file content hash is unchanged
- **THEN** the file is not selected for file-local review unless a force-wide
  trigger applies

#### Scenario: changed file is selected by scheduled audit

- **GIVEN** the audit ledger records a different content hash for a tracked file
- **WHEN** a scheduled audit computes scope
- **THEN** the file is selected and the output includes the reason it was selected

#### Scenario: mapping file changes

- **WHEN** `docs/SPEC-MAPPING.md`, `docs/IMPLEMENTATION-MATRIX.toml`, audit tooling,
  or repository-wide agent instructions change
- **THEN** the scope tooling expands the audit to the impacted Feature IDs or, when
  impact cannot be resolved deterministically, reports that a wider review is
  required

#### Scenario: new workspace crate is added

- **WHEN** a new workspace member is added under `crates/` or another tracked
  workspace path
- **THEN** the audit-scope tooling classifies its source files and selects changed
  codec-facing files for audit without needing a hardcoded crate-name update

### Requirement: reviewable audit state

The repository SHALL persist AV2 conformance audit state as reviewable generated
metadata containing at least the audit protocol version, audited commit, tracked
file paths, content hashes, impacted Feature IDs when known, and outcome. Audit
state updates SHALL be deterministic and SHALL NOT be hand-edited.

#### Scenario: audit state is regenerated

- **WHEN** the audit-scope tooling updates the ledger after a completed audit
- **THEN** rerunning the command on the same tree produces the same ledger content

#### Scenario: audit state is missing

- **WHEN** the audit-scope tooling runs without an existing ledger
- **THEN** it treats the run as a bootstrap audit and selects all in-scope files
