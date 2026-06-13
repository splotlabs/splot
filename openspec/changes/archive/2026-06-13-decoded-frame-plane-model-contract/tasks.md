## 1. Planning and Scope

- [x] 1.1 Record orchestrator plan, planning subagents, and local-reference boundary in `agent-log.md`.
- [x] 1.2 Validate the OpenSpec proposal, design, and decoder-support delta with `openspec validate decoded-frame-plane-model-contract --strict`.
- [x] 1.3 Create the implementation branch only after the OpenSpec change validates.

## 2. Docs and Matrix

- [x] 2.1 Update `docs/DECODER-ROADMAP.md` with the decoded frame and plane model contract.
- [x] 2.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` so `decoded-frame-plane-model` has Feature ID `DOC-DECODED-FRAME-PLANE-MODEL-CONTRACT`, partial status, self-contained proof, and contract-only notes.
- [x] 2.3 Add or update `docs/IMPLEMENTATION-MATRIX.toml` for `DOC-DECODED-FRAME-PLANE-MODEL-CONTRACT` without claiming runtime implementation.
- [x] 2.4 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md`.

## 3. OpenSpec Sync and Verification

- [x] 3.1 Update `openspec/specs/decoder-support/spec.md` through archive sync.
- [x] 3.2 Run focused checks: `openspec validate --all --no-interactive`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, `cargo xtask check-diagnostic-registry`, and `git diff --check`.
- [x] 3.3 Run `cargo xtask ci`.

## 4. Review, Archive, and PR

- [x] 4.1 Run required review subagents: reviewer, security-reviewer, spec-conformance-reviewer, and encoder-impact-reviewer.
- [x] 4.2 Resolve or record every review finding in `agent-log.md`.
- [x] 4.3 Archive the OpenSpec change with `openspec archive decoded-frame-plane-model-contract --yes`.
- [x] 4.4 Re-run focused checks and `cargo xtask ci` after archive.
- [ ] 4.5 Commit, push, open PR, wait for CI/review, and merge only when green.
