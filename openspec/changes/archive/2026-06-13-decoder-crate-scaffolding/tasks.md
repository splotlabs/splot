## 1. Planning and Scope

- [x] 1.1 Record planning subagents, maintainer approval, Feature ID, and local-reference boundary in `agent-log.md`.
- [x] 1.2 Validate the OpenSpec proposal, design, and spec deltas with `openspec validate decoder-crate-scaffolding --strict`.
- [x] 1.3 Create the implementation branch before source edits; validation still
  completed before implementation work.

## 2. Crate Scaffolding and Automation

- [x] 2.1 Add `crates/splot-recon` and `crates/splot-decode` as minimal workspace library crates with SPDX headers, crate docs, and workspace lint inheritance.
- [x] 2.2 Update the root workspace manifest and internal dependency-direction rules for the approved crate graph.
- [x] 2.3 Keep `splot-cli` behavior unchanged and avoid new unused dependencies or placeholder public APIs.
- [x] 2.4 Keep the validator coverage threshold scoped by updating the matching local/CI ignore regex for the new non-validator crates.

## 3. Docs and Matrix

- [x] 3.1 Update `AGENTS.md` and `docs/ARCHITECTURE.md` with the approved dependency map.
- [x] 3.2 Update decoder roadmap and diagnostics docs to distinguish scaffolded crates from emitted runtime diagnostics.
- [x] 3.3 Add decoder support and implementation matrix coverage for `INFRA-DECODER-CRATE-SCAFFOLDING`.
- [x] 3.4 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md`.

## 4. Verification

- [x] 4.1 Run focused checks: `cargo check -p splot-recon --locked`, `cargo check -p splot-decode --locked`, `cargo xtask check-dependency-direction`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, `openspec validate --all --no-interactive`, and `git diff --check`.
- [x] 4.2 Run `cargo xtask ci`.

## 5. Review, Archive, and PR

- [x] 5.1 Run required review subagents: reviewer, security-reviewer, spec-conformance-reviewer, and encoder-impact-reviewer.
- [x] 5.2 Resolve or record every review finding in `agent-log.md`.
- [x] 5.3 Archive the OpenSpec change with `openspec archive decoder-crate-scaffolding --yes`.
- [x] 5.4 Re-run focused checks and `cargo xtask ci` after archive.
- [ ] 5.5 Commit, push, open PR, wait for CI/review, and merge only when green.
