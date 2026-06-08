## Why

`splot` already has strong CI and review gates, but it lacks a durable protocol
for periodically auditing agent guidance and AV2 implementation claims after the
tree changes. This matters now because AV2 spec fidelity is the central project
risk, and a freeform audit prompt is not enough to enforce changed-file scoping or
repeatable evidence.

## What Changes

- Add a documentation audit protocol for `AGENTS.md`, assistant integration files,
  and project-authored docs, adapted from the attached knowledge-base audit guide.
- Add a heavy AV2 conformance audit protocol that coordinates multiple reviewers
  over changed files and their impacted Feature IDs, spec sections, matrix rows,
  tests, and diagnostics across current and future crates/modules.
- Add deterministic audit-scope tooling so scheduled audits can skip files whose
  content has not changed since the last completed audit, while still forcing
  broader review when core mapping files change.
- Add audit-state persistence for file hashes, audited commit, impacted Feature
  IDs, and outcomes.
- Add concise repo guidance that points agents at the audit skills without
  expanding `AGENTS.md` into the full protocol.
- Expose the audit skills through the project skill directories used by the
  supported agents, rather than relying on a single unproven universal path.
- Non-goals: do not implement new AV2 syntax, diagnostics, parser behavior,
  encoder behavior, AVM differential execution, or automatic audit PR merging in
  this change.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `process`: add repository requirements for documentation and AV2 conformance
  audit protocols. Proposed Feature IDs: `DOC-AUDIT-PROTOCOLS` for the repo-local
  audit skills and guidance, and `XTASK-AUDIT-SCOPE` for deterministic changed-file
  audit selection and state. The implementation must add matrix rows before these
  IDs are used in code or docs.

## Impact

- Affected docs and assistant integration files:
  - `AGENTS.md`
  - `.codex/skills/splot-doc-audit/SKILL.md`
  - `.codex/skills/splot-av2-conformance-audit/SKILL.md`
  - `.claude/skills/splot-doc-audit/SKILL.md`
  - `.claude/skills/splot-av2-conformance-audit/SKILL.md`
  - optionally matching `.github/skills/` or prompt files if GitHub-hosted agent
    exposure is needed
- Affected automation:
  - `xtask` command or script for audit scope/state
  - an audit-state file under a project-owned path such as `docs/audits/`
- Affected canonical tracking:
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md` if matrix rows are added
- No new third-party dependencies are expected.
