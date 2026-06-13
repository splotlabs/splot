## 1. Planning and Scope

- [x] 1.1 Record planning subagents, feature ID, and local-reference boundary in `agent-log.md`.
- [x] 1.2 Validate the OpenSpec proposal, design, and decoder-support delta with `openspec validate local-reference-evidence-manifest-contract --strict`.
- [x] 1.3 Create the implementation branch only after the OpenSpec change validates.

## 2. Manifest Contract and Checker

- [x] 2.1 Add the canonical local-reference evidence manifest skeleton at `docs/LOCAL-REFERENCE-EVIDENCE.toml`.
- [x] 2.2 Add a standalone `xtask/src/reference_evidence.rs` metadata checker that parses the manifest without running external tools.
- [x] 2.3 Validate manifest version, evidence IDs, Feature IDs, decoder-support rows, repo-relative fixture paths, fixture SHA-256 and byte lengths, digest metadata, equality assertions, and local-path leakage.
- [x] 2.4 Wire the checker into `cargo xtask check-decoder-support` and `cargo xtask ci` without adding dependencies or external decoder execution.

## 3. Tests

- [x] 3.1 Add positive unit tests showing a valid manifest with committed fixture metadata validates without external tools.
- [x] 3.2 Add negative unit tests for duplicate evidence IDs, invalid Feature IDs, invalid decoder-support rows, local or absolute paths, shell command summaries, stale fixture hashes, stale fixture lengths, malformed digests, and broken digest assertions.

## 4. Docs and Matrix

- [x] 4.1 Update `docs/DECODER-ROADMAP.md` with the manifest contract and Stage 10 status.
- [x] 4.2 Add decoder support and implementation matrix coverage for `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST` without claiming runtime decode support or live AVM/dav2d execution.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md`.
- [x] 4.4 Update focused docs if needed to distinguish the decoder reference evidence manifest from the validator conformance manifest.

## 5. Verification

- [x] 5.1 Run focused checks: `cargo test -p xtask reference_evidence --locked`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, `openspec validate --all --no-interactive`, and `git diff --check`.
- [x] 5.2 Run `cargo xtask ci`.

## 6. Review, Archive, and PR

- [x] 6.1 Run required review subagents: reviewer, security-reviewer, spec-conformance-reviewer, and encoder-impact-reviewer.
- [x] 6.2 Resolve or record every review finding in `agent-log.md`.
- [x] 6.3 Archive the OpenSpec change with `openspec archive local-reference-evidence-manifest-contract --yes`.
- [x] 6.4 Re-run focused checks and `cargo xtask ci` after archive.
- [ ] 6.5 Commit, push, open PR, wait for CI/review, and merge only when green.
