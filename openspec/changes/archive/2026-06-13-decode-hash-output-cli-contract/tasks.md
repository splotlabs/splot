## 1. Planning and Scope

- [x] 1.1 Record orchestrator plan, planning subagents, and local-reference boundary in `agent-log.md`.
- [x] 1.2 Validate the OpenSpec proposal, design, and decoder-support delta with `openspec validate decode-hash-output-cli-contract --strict`.
- [x] 1.3 Create the implementation branch only after the OpenSpec change validates.

## 2. CLI Implementation

- [x] 2.1 Add `--output-format <y4m|hash>` and CLI-only output target resolution to `crates/splot-cli/src/commands/decode.rs`.
- [x] 2.2 Preserve the existing unsupported diagnostic and no-read/no-touch runtime behavior for every valid decode parse.
- [x] 2.3 Add CLI tests for hash mode, explicit Y4M mode, missing output selection, and unchanged diagnostic rendering.

## 3. Docs and Matrix

- [x] 3.1 Update `docs/DECODER-ROADMAP.md` with the selected hash-output CLI contract.
- [x] 3.2 Add decoder support and implementation matrix coverage for `CLI-DECODE-HASH-OUTPUT` without claiming runtime decode support.
- [x] 3.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md`.

## 4. Verification

- [x] 4.1 Run focused checks: CLI decode tests, `openspec validate --all --no-interactive`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, `cargo xtask check-diagnostic-registry`, and `git diff --check`.
- [x] 4.2 Run `cargo xtask ci`.

## 5. Review, Archive, and PR

- [x] 5.1 Run required review subagents: reviewer, security-reviewer, spec-conformance-reviewer, and encoder-impact-reviewer.
- [x] 5.2 Resolve or record every review finding in `agent-log.md`.
- [x] 5.3 Archive the OpenSpec change with `openspec archive decode-hash-output-cli-contract --yes`.
- [x] 5.4 Re-run focused checks and `cargo xtask ci` after archive.
- [ ] 5.5 Commit, push, open PR, wait for CI/review, and merge only when green.
