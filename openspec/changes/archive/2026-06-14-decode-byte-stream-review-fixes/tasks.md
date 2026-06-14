## 1. OpenSpec and Archive Hygiene

- [x] 1.1 Sync and archive the completed `decode-byte-stream-planner` change.
- [x] 1.2 Create and validate `decode-byte-stream-review-fixes` artifacts.
- [x] 1.3 Keep `agent-log.md` updated with subagent outputs, implementation
      notes, and review findings.

## 2. Review Fix Implementation

- [x] 2.1 Preserve unsupported-structure precedence during bounded raw byte
      traversal.
- [x] 2.2 Keep `IvfFrameCursor` state unchanged on fatal frame-header errors.
- [x] 2.3 Add `decode_plan_bytes`-specific prefixed CI fuzz seeds.
- [x] 2.4 Update `DecodeContext` docs for raw-byte planning.

## 3. Tests and Gates

- [x] 3.1 Add focused regression tests for unsupported-before-limit precedence.
- [x] 3.2 Add focused regression tests for IVF cursor retry behavior.
- [x] 3.3 Run focused Rust/OpenSpec/fuzz-target checks.
- [x] 3.4 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`,
      and `cargo xtask ci`.

## 4. Review and PR

- [x] 4.1 Run required subagent review passes and resolve findings.
- [ ] 4.2 Open a ready PR that explicitly says it addresses PR #113 review
      `4492663492`.
- [ ] 4.3 Request Codex review on the PR and do not merge until the latest head
      has explicit Codex no-findings/thumbs-up/final sign-off.
