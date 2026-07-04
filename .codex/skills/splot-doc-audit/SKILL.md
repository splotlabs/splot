---
name: splot-doc-audit
description: Audit splot project-authored documentation and agent guidance for stale claims, broken paths, duplicated rules, contradictions, misplaced guidance, and size drift. Use for scheduled or on-demand documentation audits, not for AV2 implementation conformance review.
license: PolyForm-Noncommercial-1.0.0
---

# splot Documentation Audit

Use this skill for `DOC-AUDIT-PROTOCOLS` documentation audits. This is a
documentation and agent-guidance audit, not an implementation audit.

## Scope

Audit project-authored guidance and documentation:

- `AGENTS.md`, `CLAUDE.md`, and `.github/copilot-instructions.md`
- `.codex/skills/`, `.claude/commands/`, `.claude/skills/`, `.github/skills/`,
  and `.github/prompts/`
- project docs under `docs/`, excluding the AV2 spec mirror body
- OpenSpec process docs and active change artifacts when they make repository
  process claims

Treat `docs/spec/av2/<version>/` as read-only third-party evidence. Do not
hand-edit it and do not copy spec text into project-authored docs.

## Procedure

1. Run `git status --short` and preserve user work.
2. Build a list of concrete claims: file paths, commands, tool versions, Feature
   IDs, spec mirror citations, assistant-integration paths, ownership claims, and
   "must/never" rules.
3. Verify claims against the live repo using `rg`, `rg --files`, `git ls-files`,
   `cargo xtask feature-status`, and source-of-truth files.
4. Flag duplicate rules and recommend one canonical home.
5. Flag cross-file contradictions for human review; do not pick a winner unless
   the source of truth is explicit.
6. If one file contradicts itself, treat that file as blocking: do not edit or
   re-stamp it; report the conflict.
7. Flag wrong-file-fit rules and recommend a target file rather than moving them
   automatically.
8. Flag stale, vague, redundant, or obvious guidance in recommendations. Do not
   delete such rules without explicit user approval.
9. Flag files over 300 lines or 15 KB with proposed split points.

## Allowed Edits

The audit may propose or apply small documentation-only fixes when requested:

- broken repo paths
- outdated command names
- stale Feature IDs after confirming the matrix source of truth
- duplicated guidance where the better home is unambiguous
- audit comments for unclear claims

Do not edit production Rust code, generated files, lockfiles, vendored material,
the AV2 spec mirror body, or behavior documentation that depends on ambiguous AV2
spec interpretation.

## Output

Return a concise report or a documentation-only PR body with:

- blocking in-file contradictions
- summary of claims checked
- small fixes made or proposed
- recommendations requiring human judgment
- evidence links or command outputs for every changed claim

Never auto-merge. If zero issues are found, report the number of files and claims
checked.
