# Change: comment-density-ratchet

## Feature IDs

- `INFRA-COMMENT-DENSITY-RATCHET`

## Why

The source tree has accumulated redundant implementation comments: control-flow
narration, historical notes, copied AV2 prose, and process markers. Those comments
increase review cost and drift risk without improving the code.

This change applies the Comment Diet mission: remove redundant implementation
comments, keep required Rustdoc and explicit policy markers, and add a permanent
comment-density ratchet so the removed noise cannot silently return.

## Scope

- Spec sections: none. This is repository tooling and source hygiene.
- Crates/modules: source comments under `crates/**/*.rs`, `xtask/**/*.rs`, and
  `fuzz/fuzz_targets/**/*.rs`; a new `xtask/src/comment_density.rs` gate.
- Docs/CI: `docs/agents/coding-standards.md`, `docs/CODE_REVIEW.md`,
  `docs/agents/commands.md`, `tools/comments/budget.toml`,
  `.github/workflows/ci.yml`, and the implementation matrix.

## Non-goals

- No AV2 semantic changes.
- No parser, validator, decoder, encoder, reconstruction, or CLI behavior changes.
- No Cargo dependency changes.
- No hand edits to generated AV2 table data.
- No weakening of existing CI, lint, fuzz, diagnostic-registry, source-line,
  zero-copy, concurrency, or duplication gates.

## Acceptance criteria

- [x] Baseline and post-cleanup implementation-comment counts are recorded.
- [x] Implementation comments are reduced by at least 10x, excluding SPDX and
      public Rustdoc.
- [x] `cargo xtask check-comment-density` exists, is unit-tested, and enforces
      `tools/comments/budget.toml`.
- [x] The check runs inside `cargo xtask ci` and GitHub CI.
- [x] Agent and review docs record the Comment Diet policy.
- [x] `cargo xtask check-feature-status` passes.
- [x] `cargo xtask ci` passes.
