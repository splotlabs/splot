## 1. Planning and OpenSpec

- [x] 1.1 Record orchestrator plan, required subagents, PR #101 concurrency boundary, and AVM/dav2d local-only boundary in `agent-log.md`.
- [x] 1.2 Validate proposal, design, and delta spec with `openspec validate local-reference-evidence-entries --strict`.

## 2. Manifest Entries

- [x] 2.1 Add checked evidence entries to `docs/LOCAL-REFERENCE-EVIDENCE.toml` for the 8-bit and 10-bit committed intra IVF fixtures with fixture SHA-256/length metadata.
- [x] 2.2 Record only sanitized AVM/dav2d reference metadata: bare executable names, upstream revisions, version summaries, command summaries, raw MD5 digests, output scopes, and equality assertions.
- [x] 2.3 Run `cargo xtask check-reference-evidence` and keep the checker external-tool-free.

## 3. Docs and Matrix

- [x] 3.1 Update `docs/DECODER-ROADMAP.md` to describe the non-empty evidence manifest without claiming runtime decode/hash/Y4M support.
- [x] 3.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` for the local-reference evidence manifest and deterministic-frame-hash rows.
- [x] 3.3 Update `docs/IMPLEMENTATION-MATRIX.toml` feature notes/proof for the evidence entries.
- [x] 3.4 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md` as needed.

## 4. Verification

- [x] 4.1 Run focused checks: `cargo xtask check-reference-evidence`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, `openspec validate --all --no-interactive`, and `git diff --check`.
- [x] 4.2 Run `cargo xtask ci`.

## 5. Review, Archive, and PR

- [x] 5.1 Run required review subagents: reviewer, security-reviewer, spec-conformance-reviewer, and encoder-impact-reviewer.
- [x] 5.2 Resolve or record every review finding in `agent-log.md`.
- [x] 5.3 Archive the OpenSpec change with `openspec archive local-reference-evidence-entries --yes`.
- [x] 5.4 Re-run focused checks and `cargo xtask ci` after archive.
- [ ] 5.5 Commit, push, open a ready PR, wait for CI and latest-head Codex review, and merge only when all gates are green.
