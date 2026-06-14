# process Specification

## Purpose

Repository process guardrails for source provenance, review hygiene, and CI-enforced
contribution requirements.

Tracked by Feature IDs: `DOC-ENCODER-REFERENCE-GATE`,
`XTASK-CONVENTIONAL-COMMITS`, `DOC-AUDIT-PROTOCOLS`,
`XTASK-AUDIT-SCOPE`, `XTASK-SOURCE-LINES`.
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

### Requirement: AV2 spec grounding via the committed mirror

Development that asserts AV2 syntax, constants, tables, or semantics SHALL ground
those claims in the committed AV2 specification mirror under `docs/spec/av2/<version>/`,
treating it as the canonical offline source of truth alongside the upstream AOM
PDF/HTML. Contributors and agents SHALL NOT invent spec behavior; where a detail
is intentionally unmodeled, the existing `TODO(spec: <FEATURE-ID>)` convention
applies. Tracked by `DOC-AV2-SPEC-MIRROR`.

#### Scenario: a change cites AV2 behavior

- **WHEN** a code comment, diagnostic, or document states an AV2 syntax element,
  constant, table, or semantic rule
- **THEN** it is traceable to a `§` section resolvable in the committed mirror
  (via `index.md`), not to memory or an uncited external source

#### Scenario: spec text is needed offline

- **WHEN** an agent or reviewer needs the exact normative wording of an AV2
  section while working in the repository
- **THEN** the text is available from the committed mirror without network access

### Requirement: documentation audit protocol

The repository SHALL provide a documentation audit protocol that checks
project-authored agent guidance and documentation for stale claims, broken paths,
duplicated rules, contradictions, size drift, and misplaced guidance. The protocol
SHALL treat the AV2 specification mirror as read-only third-party material and
SHALL NOT edit production Rust code. Tracked by `DOC-AUDIT-PROTOCOLS`.

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
paths. The repository SHALL NOT rely on `.agents/skills/` as the only project
skill location. Tracked by `DOC-AUDIT-PROTOCOLS`.

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
the workspace. Tracked by `DOC-AUDIT-PROTOCOLS`.

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
crates and docs present when the tool was first written. Tracked by
`XTASK-AUDIT-SCOPE`.

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
state updates SHALL be deterministic and SHALL NOT be hand-edited. Tracked by
`XTASK-AUDIT-SCOPE`.

#### Scenario: audit state is regenerated

- **WHEN** the audit-scope tooling updates the ledger after a completed audit
- **THEN** rerunning the command on the same tree produces the same ledger content

#### Scenario: audit state is missing

- **WHEN** the audit-scope tooling runs without an existing ledger
- **THEN** it treats the run as a bootstrap audit and selects all in-scope files

### Requirement: validator diagnostic registry enforcement

The repository SHALL enforce that `docs/VALIDATOR-DIAGNOSTICS.md` lists exactly the
diagnostic rule-ID literals present in `crates/splot-validate/src`. A
`cargo xtask check-diagnostic-registry` gate, run as part of `cargo xtask ci`, SHALL extract
the rule-ID literals from non-test, non-comment validator source and compare them against the
IDs documented in the file's enforced registry region. The gate SHALL fail when an emitted ID
is undocumented or when the registry documents an ID that is not present in the source.
Tracked by `XTASK-DIAGNOSTIC-REGISTRY`.

#### Scenario: emitted rule ID missing from the registry

- **WHEN** the validator source contains a rule-ID literal that is absent from the registry region
- **THEN** `cargo xtask check-diagnostic-registry` fails and names the undocumented ID

#### Scenario: registry lists an ID not present in source

- **WHEN** the registry region documents a rule ID that does not appear as a literal in non-test validator source
- **THEN** `cargo xtask check-diagnostic-registry` fails and names the unemitted ID

#### Scenario: registry matches the source

- **WHEN** the documented registry IDs equal the rule-ID literals in non-test, non-comment validator source
- **THEN** `cargo xtask check-diagnostic-registry` passes

#### Scenario: registry-only check identifiers are documented

- **WHEN** the validator source contains `Check::id()` registry identifiers (the `<ns>/syntax` literals) that are routed through `syntax_error_diagnostic()` rather than emitted verbatim
- **THEN** those identifiers are documented in a labeled registry sub-table so the documented set still equals the extracted set

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
not match its render. Tracked by `XTASK-FEATURE-STATUS`.

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

### Requirement: strict documentation build gate

The repository SHALL build rustdoc documentation for the whole workspace with
warnings denied (`RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps
--locked`) as a blocking step in both `cargo xtask ci` and the CI `ci` job. A
rustdoc warning or error in any workspace crate SHALL fail the gate.

#### Scenario: rustdoc warning blocks the gate

- **WHEN** a workspace crate contains a doc comment that rustdoc reports on
  (for example an unresolved or private intra-doc link)
- **THEN** `cargo xtask ci` and the CI `ci` job fail at the docs step

#### Scenario: clean docs pass the gate

- **WHEN** `cargo doc --workspace --no-deps --locked` emits no warnings under
  `RUSTDOCFLAGS=-D warnings`
- **THEN** the docs step passes in both `cargo xtask ci` and the CI `ci` job

### Requirement: blocking validator coverage threshold

CI SHALL measure workspace line coverage with `cargo llvm-cov` and SHALL fail
the coverage job when line coverage over the `crates/splot-validate` sources,
in isolation, is below 90 percent. The job SHALL NOT be marked
`continue-on-error`. The workspace-wide summary and the lcov artifact SHALL
continue to be produced. `cargo xtask coverage` SHALL enforce the same
threshold locally when `cargo-llvm-cov` is installed.

#### Scenario: validator coverage regression blocks the merge

- **WHEN** a change drops `crates/splot-validate` line coverage below 90
  percent
- **THEN** the CI coverage job fails and the PR cannot merge

#### Scenario: other crates do not gate

- **WHEN** line coverage outside `crates/splot-validate` changes
- **THEN** the threshold check is unaffected (only `splot-validate` files are
  in the gated report scope)

### Requirement: OpenSpec validation in the local gate

`cargo xtask ci` SHALL run `openspec validate --all --no-interactive` when the
`openspec` binary is available, under the same run-if-present policy as the
other external-tool checks (skip with an install hint when absent), so the
local gate and the CI workflow's conditional OpenSpec step enforce the same
validation.

#### Scenario: openspec installed

- **WHEN** `cargo xtask ci` runs on a machine with `openspec` on PATH and a
  spec or active change fails validation
- **THEN** the gate fails at the OpenSpec step

#### Scenario: openspec absent

- **WHEN** `cargo xtask ci` runs on a machine without `openspec`
- **THEN** the step is skipped with an install hint and the gate continues

### Requirement: Rust source-file size budget

The repository SHALL document a soft Rust source-file budget of 1000 physical
lines and provide `cargo xtask check-source-lines` to inspect tracked and
non-ignored new `.rs` files offline. The check SHALL print advisory warnings for
files above the soft budget and SHALL fail when a non-exempt Rust file exceeds
the configured hard cap. `cargo xtask ci` SHALL run the check. Tracked by
`XTASK-SOURCE-LINES`.

#### Scenario: Rust file exceeds the soft budget only

- **WHEN** a checked Rust source file has more than 1000 physical lines but does
  not exceed the hard cap
- **THEN** `cargo xtask check-source-lines` prints an advisory warning and exits
  successfully

#### Scenario: Rust file exceeds the hard cap

- **WHEN** a checked Rust source file exceeds the hard cap and has no documented
  exception
- **THEN** `cargo xtask check-source-lines` fails and names the offending file

#### Scenario: source-line check runs in CI

- **WHEN** `cargo xtask ci` runs
- **THEN** it also runs `cargo xtask check-source-lines` as a deterministic
  repository check

### Requirement: generated decoder support document
The repository SHALL provide a generated document
`docs/DECODER-SUPPORT-STATUS.md`, rendered from
`docs/DECODER-SUPPORT-MATRIX.toml` by
`cargo xtask decoder-support --format markdown --output docs/DECODER-SUPPORT-STATUS.md`.
`cargo xtask check-decoder-support` SHALL fail when the committed document does
not match its render. `cargo xtask ci` SHALL run this check without invoking
AVM, dav2d, or any external decoder. Tracked by
`XTASK-DECODER-SUPPORT-STATUS`.

#### Scenario: looking up decoder support
- **WHEN** a reader opens `docs/DECODER-SUPPORT-STATUS.md`
- **THEN** the document shows decoder/reconstruction row status counts and the
  row-level support status rendered from `docs/DECODER-SUPPORT-MATRIX.toml`

#### Scenario: committed decoder status drifts
- **WHEN** `docs/DECODER-SUPPORT-MATRIX.toml` changes without regenerating
  `docs/DECODER-SUPPORT-STATUS.md`
- **THEN** `cargo xtask check-decoder-support` fails and names the regenerate
  command

#### Scenario: reference tools are absent
- **WHEN** `cargo xtask ci` runs on a machine without AVM or dav2d
- **THEN** the decoder support document check still runs from committed files
  only and does not locate, build, or execute either reference tool

### Requirement: decoder diagnostic registry enforcement

The repository SHALL enforce that `docs/DECODER-DIAGNOSTICS.md` lists exactly
the emitted `decode/*` diagnostic `rule_id` literals present in current decoder
emission source roots. `cargo xtask check-diagnostic-registry`, run as part of
`cargo xtask ci`, SHALL compare the emitted decoder `rule_id` set against the
marker-delimited registry region and fail on drift in either direction. The
gate SHALL reject diagnostic-looking rule IDs in decoder emission roots or the
decoder registry when they do not use the `decode/*` namespace. Tracked by
`XTASK-DECODER-DIAGNOSTIC-REGISTRY`.

#### Scenario: emitted decoder rule ID missing from the registry

- **WHEN** a decoder emission source contains a `decode/*` rule-ID literal that
  is absent from the decoder registry region
- **THEN** `cargo xtask check-diagnostic-registry` fails and names the
  undocumented ID

#### Scenario: decoder registry lists an ID not present in source

- **WHEN** the decoder registry region documents a `decode/*` rule ID that does
  not appear in decoder emission source
- **THEN** `cargo xtask check-diagnostic-registry` fails and names the unemitted
  ID

#### Scenario: decoder registry matches source

- **WHEN** the documented decoder registry IDs equal the emitted decoder rule-ID
  literals
- **THEN** `cargo xtask check-diagnostic-registry` passes the decoder registry
  check

#### Scenario: decoder rule ID uses another namespace

- **WHEN** a decoder emission source or the decoder registry contains a
  diagnostic-looking rule ID outside the `decode/*` namespace
- **THEN** `cargo xtask check-diagnostic-registry` fails and names the
  unsupported namespace

### Requirement: Decoder crate dependency direction
The repository SHALL enforce the approved decoder/reconstruction dependency
boundary through `cargo xtask check-dependency-direction`. `splot-recon` SHALL
depend on no other `splot-*` crate. `splot-decode` MAY depend on `splot-core`
and `splot-recon` when implementation code needs those crates. `splot-cli` MAY
depend on `splot-decode` for library-owned decoder diagnostics and future CLI
decode integration. `splot-encode` MAY depend on `splot-recon` only through a
future encoder/reconstruction API change.

#### Scenario: Approved decoder graph is accepted

- **WHEN** `cargo xtask check-dependency-direction` runs
- **THEN** the allow-list includes `splot-recon` and `splot-decode`
- **AND** any internal dependency outside the approved graph is rejected

#### Scenario: Coverage threshold stays validator-scoped

- **WHEN** the workspace gains `splot-recon` and `splot-decode`
- **THEN** local and CI coverage threshold commands keep gating
  `crates/splot-validate` line coverage only
- **AND** the new scaffold crates do not accidentally join the validator
  coverage threshold

### Requirement: conflict-zone guard

The workspace SHALL provide a `cargo xtask check-conflict-zone` command that
compares the working branch's committed diff against `main` (merge-base relative)
to a committed denylist of decoder-owned paths, and SHALL exit non-zero when any
changed path falls inside the denylist. The denylist SHALL cover
`crates/splot-decode/**`, `crates/splot-recon/**`, `docs/DECODER-*`,
`docs/LOCAL-REFERENCE-EVIDENCE.toml`, `fuzz/fuzz_targets/decode*`,
`crates/splot-cli/src/commands/decode.rs`, and new AVM/dav2d integration paths
under the workspace code/build roots. The command SHALL be folded into
`cargo xtask ci` and run as a step in CI.

#### Scenario: a validator change stays clear of the conflict zone

- **WHEN** the diff vs `main` touches only validator/inspector/tooling files
- **THEN** `cargo xtask check-conflict-zone` exits zero with an `ok` notice

#### Scenario: a change touches a decoder-owned path

- **WHEN** the diff vs `main` creates, edits, or deletes any denylisted path
- **THEN** `cargo xtask check-conflict-zone` prints each offending path and exits
  non-zero

### Requirement: conflict-zone guard is decoder-safe

The guard SHALL NOT break the decoder stream or fail spuriously. It SHALL skip
with a notice (returning success) when no `main` base is resolvable, when the diff
is empty, when the current branch is a decoder-stream branch (its name contains
`decode` or `recon`), or when `SPLOT_SKIP_CONFLICT_ZONE=1` is set.

#### Scenario: decoder-stream branch is exempt

- **WHEN** the guard runs on a branch whose name contains `decode` or `recon`
- **THEN** it skips with a notice and exits zero without inspecting the diff

#### Scenario: no base to compare against

- **WHEN** no `main` base is resolvable (e.g. a shallow clone) or the diff is empty
- **THEN** the guard skips with a notice and exits zero

