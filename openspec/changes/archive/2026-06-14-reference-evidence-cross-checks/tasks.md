## 1. Planning and OpenSpec

- [x] 1.1 Record orchestrator plan, PR #101 concurrency boundary, AVM/dav2d local-only boundary, and required subagents in `agent-log.md`.
- [x] 1.2 Validate proposal, design, and delta spec with `openspec validate reference-evidence-cross-checks --strict`.

## 2. Checker Implementation

- [x] 2.1 Add xtask-internal manifest metadata helpers that expose evidence IDs and their `decoder_support_rows` without invoking external tools.
- [x] 2.2 Extend `check-decoder-support` validation so canonical manifest pointers resolve to manifest entries.
- [x] 2.3 Reject non-reciprocal links when a cited evidence entry does not list the citing matrix row.

## 3. Tests and Docs

- [x] 3.1 Add positive unit coverage for valid reciprocal manifest pointers.
- [x] 3.2 Add negative unit coverage for missing manifest evidence IDs.
- [x] 3.3 Add negative unit coverage for non-reciprocal manifest-to-row links.
- [x] 3.4 Update docs or generated status only if validation behavior changes visible output.

## 4. Verification

- [x] 4.1 Run focused checks: `cargo test -p xtask reference_evidence --locked`, `cargo test -p xtask decoder_support --locked`, `cargo xtask check-reference-evidence`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, `openspec validate --all --no-interactive`, and `git diff --check`.
- [x] 4.2 Run `cargo xtask ci`.

## 5. Review, Archive, and PR

- [x] 5.1 Run required review subagents: reviewer, security-reviewer, spec-conformance-reviewer, and encoder-impact-reviewer.
- [x] 5.2 Resolve or record every review finding in `agent-log.md`.
- [x] 5.3 Archive the OpenSpec change with `openspec archive reference-evidence-cross-checks --yes`.
- [x] 5.4 Re-run focused checks and `cargo xtask ci` after archive.
- [ ] 5.5 Commit, push, open a ready PR, wait for CI and latest-head Codex review, and merge only when all gates are green.
