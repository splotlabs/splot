# Tasks

## Baseline

- [x] Measure pre-cleanup implementation comments.
- [x] Measure post-cleanup implementation comments.

## Tracking and docs

- [x] Add `INFRA-COMMENT-DENSITY-RATCHET` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Record this change in `openspec/changes/README.md`.
- [x] Add the `Comment Diet` policy to `docs/agents/coding-standards.md`.
- [x] Add comment-review prompts to `docs/CODE_REVIEW.md`.
- [x] Document `check-comment-density` in `docs/agents/commands.md`.

## Gate

- [x] Add `tools/comments/budget.toml`.
- [x] Add `cargo xtask check-comment-density`.
- [x] Wire `check-comment-density` into `cargo xtask ci`.
- [x] Run the gate in `.github/workflows/ci.yml`.
- [x] Add unit tests for comment counting and threshold enforcement.

## Cleanup

- [x] Remove redundant implementation comments from source files.
- [x] Preserve SPDX, generated markers, required Rustdoc, tracked TODOs, and
      zero-copy copy markers.
- [x] Search for prohibited process-history wording after cleanup.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo test --doc --workspace --locked`
- [x] `cargo doc --workspace --no-deps --locked`
- [x] `cargo xtask check-comment-density`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
