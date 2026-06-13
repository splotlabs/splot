## 1. Planning

- [x] 1.1 Create proposal, design, and decoder-support delta specs for the contract-only change.
- [x] 1.2 Record planning subagent outputs in `agent-log.md`.
- [x] 1.3 Run `openspec validate decoder-limits-contract --strict`.
- [x] 1.4 Create the feature branch only after the OpenSpec change validates.

## 2. Documentation And Matrices

- [x] 2.1 Update `docs/DECODER-ROADMAP.md` with the decode limits contract and planned diagnostic shape.
- [x] 2.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` so `decode-limits-budget` has Feature ID `DOC-DECODE-LIMITS-CONTRACT`, partial status, self-contained proof, and planned diagnostics.
- [x] 2.3 Document `decode/resource-limit` as planned text without adding it to the emitted diagnostic registry marker region.
- [x] 2.4 Add `DOC-DECODE-LIMITS-CONTRACT` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 2.5 Regenerate generated docs: decoder support status, feature status, and spec coverage.

## 3. Verification

- [x] 3.1 Run `openspec validate decoder-limits-contract --strict`.
- [x] 3.2 Run `cargo xtask check-decoder-support`.
- [x] 3.3 Run `cargo xtask check-feature-status`.
- [x] 3.4 Run `cargo xtask check-diagnostic-registry`.
- [x] 3.5 Run `cargo xtask ci`.

## 4. Review And Archive

- [x] 4.1 Run required subagent review passes and record sign-offs in `agent-log.md`.
- [x] 4.2 Resolve or explicitly close all review findings.
- [x] 4.3 Archive the OpenSpec change with `openspec archive decoder-limits-contract --yes`.
- [x] 4.4 Verify archive output and rerun the full gate.
- [ ] 4.5 Commit, push, open the PR, wait for CI and review, then merge only after all gates are green.
