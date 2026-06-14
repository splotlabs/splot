## 1. OpenSpec And Review Scope

- [x] 1.1 Create design, spec delta, and task artifacts for `decode-cli-byte-planner-handoff`.
- [x] 1.2 Record subagent findings and PR #113 review carry-forward decisions in `agent-log.md`.
- [x] 1.3 Validate the OpenSpec change.

## 2. Library Diagnostics

- [x] 2.1 Add a `splot-decode` diagnostic adapter for malformed source, resource limit, planner unsupported, and runtime unsupported reports.
- [x] 2.2 Add stable source issue kind strings without adding serde or new dependencies to `splot-decode`.
- [x] 2.3 Add library tests for the diagnostic adapter and PR #113 non-regression cases.

## 3. CLI Handoff

- [x] 3.1 Update `splot decode` to read input, construct `DecodeContext`, call `plan_bytes`, and render diagnostic reports.
- [x] 3.2 Preserve no-touch output behavior for malformed, unsupported, resource-limit, and runtime-deferral paths.
- [x] 3.3 Keep missing input and pool failures as operational errors, not `decode/*` diagnostics.

## 4. Docs And Matrices

- [x] 4.1 Update `docs/DECODER-DIAGNOSTICS.md` for emitted `decode/malformed-source` and `decode/resource-limit`.
- [x] 4.2 Update decoder support and implementation matrices, then regenerate decoder support status.
- [x] 4.3 Keep AVM/dav2d evidence marked not applicable for this slice.

## 5. Verification And PR

- [x] 5.1 Run targeted CLI/library tests and policy checks.
- [x] 5.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, and `cargo xtask ci`.
- [ ] 5.3 Open a ready PR, request Codex review, and wait for explicit latest-head Codex completion before any merge.
