# Change: enforce-conventional-commits

## Feature IDs

- `XTASK-CONVENTIONAL-COMMITS`

## Why

Commit history and pull request titles should be machine-readable enough for
changelog/release automation and easy review scanning. A documented rule without a
CI gate is too easy to miss, so the repository should reject non-conventional PR
titles and commit subjects before merge.

## Scope

- Spec sections: none (process/tooling).
- Crates/modules: `xtask/src/main.rs`, `.github/workflows/ci.yml`.
- CLI/docs/tests: `cargo xtask check-conventional-title`,
  `cargo xtask check-conventional-commits`, `AGENTS.md`, `CONTRIBUTING.md`,
  `.github/PULL_REQUEST_TEMPLATE.md`, implementation matrix status docs.

## Non-goals

- No AV2 syntax, parser, validator, encoder, or dependency-graph changes.
- No local Git hook installation.
- No generated changelog or release-note automation.

## Acceptance criteria

- [x] Implementation matrix row exists for `XTASK-CONVENTIONAL-COMMITS`.
- [x] `cargo xtask check-conventional-title` validates PR titles.
- [x] `cargo xtask check-conventional-commits` validates commit subject lines.
- [x] CI runs the check against PR and push commit ranges.
- [x] Contributor docs describe the required title/subject format and allowed types.
- [x] Positive and negative parser tests exist in `xtask`.
