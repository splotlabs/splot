# Design: Comment Density Ratchet

## Context

This change removes redundant implementation comments across Rust source under
`crates/`, `xtask/`, and `fuzz/fuzz_targets/`, then adds a permanent repository
gate so the comment volume does not drift back up. The work is process/tooling
only: it must not change AV2 semantics, parser behavior, diagnostics, codec
outputs, crate dependencies, or licensing terms.

The repo already uses `cargo xtask ci` as the acceptance surface, with ratcheted
quality gates for source lines, feature status, generated artifacts, policy
checks, and duplication. The comment-density check fits that model better than a
standalone script because it becomes part of the same local and GitHub workflow.

## Goals / Non-Goals

**Goals:**

- Count implementation comment lines in scoped Rust source files.
- Exclude required SPDX headers, public Rustdoc, generated source, and comment
  looking text inside string literals.
- Enforce a checked-in budget in `tools/comments/budget.toml`.
- Wire the check into `cargo xtask ci` and GitHub CI.
- Record the baseline and post-cleanup reduction.

**Non-Goals:**

- No AV2 behavior, syntax, table, or diagnostic changes.
- No generated AV2 table-data edits.
- No new dependencies.
- No broad refactors beyond tiny helper extraction needed to keep existing gates
  passing.

## Decisions

1. Implement the gate in `xtask`, not as an external script.
   `xtask` already owns repository policy checks and is run by both local CI and
   GitHub CI. Keeping the logic in Rust also avoids a new runtime dependency.
   Alternative considered: shell/Python scanner. Rejected because it would add a
   second policy implementation style and weaker source parsing.

2. Track an absolute implementation-comment budget.
   The cleanup target is measured in lines, and a fixed checked-in cap makes the
   ratchet easy to understand in review. Alternative considered: percentage-only
   density. Rejected because total source volume can hide comment growth.

3. Use source-aware comment scanning.
   The gate distinguishes comments from string literals, skips generated files
   marked by the project generator, and treats SPDX/Rustdoc separately. A simple
   line-prefix grep would be faster but would falsely count tests and fixtures
   containing `//` inside strings.

4. Keep the cleanup behavior-neutral.
   Most edits delete redundant comments. Where the existing duplication budget
   was exceeded after broad cleanup, the fix is small test/helper extraction
   without changing functional paths or raising the duplication budget.

## Risks / Trade-offs

- [Risk] A strict absolute budget can block small legitimate comments.
  Mitigation: keep concise invariant/spec-anchor comments allowed by policy, and
  adjust the budget only through deliberate review when a new real need appears.

- [Risk] The scanner can misclassify unusual Rust syntax.
  Mitigation: unit-test string-literal and threshold cases, and keep the scanner
  conservative about generated/Rustdoc/SPDX exclusions.

- [Risk] Large comment deletion can remove useful context.
  Mitigation: preserve Rustdoc, SPDX, generated markers, `TODO(spec)`, copy
  markers, and comments that explain invariants or spec anchors.

## Migration Plan

1. Record the pre-cleanup baseline.
2. Delete redundant implementation comments while preserving allowed categories.
3. Add `cargo xtask check-comment-density` and its budget file.
4. Wire the check into local and GitHub CI.
5. Run `cargo xtask ci`.

Rollback is ordinary git revert of the cleanup and gate files. No data migration
or runtime compatibility path is involved.

## Open Questions

None.
