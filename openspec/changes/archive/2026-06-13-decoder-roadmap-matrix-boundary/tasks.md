## 1. Planning and Scope

- [x] 1.1 Record Phase 1 subagent findings and local reference evidence in `agent-log.md`.
- [x] 1.2 Validate OpenSpec artifacts with `openspec validate decoder-roadmap-matrix-boundary --strict`.
- [x] 1.3 Confirm no dependency graph change or AVM/dav2d repo integration is in scope.

## 2. Decoder Docs and Matrix

- [x] 2.1 Add `docs/DECODER-ROADMAP.md` with mission scope, staged tiering, hashing policy, unsupported-feature contract, and local-only reference boundary.
- [x] 2.2 Add `docs/DECODER-SUPPORT-MATRIX.toml` with row ids, Feature IDs, spec sections, parser source, decode/recon module, tier, status, tests, diagnostics, reference evidence, and notes.
- [x] 2.3 Add generated `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 2.4 Update README/docs pointers without claiming pixel decode is implemented.

## 3. Automation

- [x] 3.1 Implement `cargo xtask decoder-support --format markdown --output docs/DECODER-SUPPORT-STATUS.md`.
- [x] 3.2 Implement `cargo xtask check-decoder-support` drift validation.
- [x] 3.3 Wire `check-decoder-support` into `cargo xtask ci`.
- [x] 3.4 Add focused xtask unit tests for matrix parsing, status validation, markdown rendering, and drift detection.

## 4. Feature Tracking

- [x] 4.1 Add implementation-matrix rows for `DOC-DECODER-ROADMAP`, `DOC-DECODER-SUPPORT-MATRIX`, `XTASK-DECODER-SUPPORT-STATUS`, and `CLI-DECODE`.
- [x] 4.2 Regenerate `docs/FEATURE-STATUS.md` and `docs/SPEC-COVERAGE.md`.
- [x] 4.3 Record proof commands/tests for rows whose code stages become `done`.

## 5. Review and Gates

- [x] 5.1 Run `cargo xtask feature-status`.
- [x] 5.2 Run `cargo xtask check-feature-status`.
- [x] 5.3 Run `openspec validate decoder-roadmap-matrix-boundary --strict`.
- [x] 5.4 Run `cargo xtask ci`.
- [x] 5.5 Complete subagent reviews: @reviewer, @security-reviewer, @spec-conformance-reviewer, and @encoder-impact-reviewer.
- [x] 5.6 Record final AVM/dav2d boundary review result in `agent-log.md`.
