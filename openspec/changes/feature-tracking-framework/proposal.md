# Change: feature-tracking-framework

## Feature IDs

- `XTASK-FEATURE-STATUS`
- `DOC-FEATURE-TRACKING`

## Why

AV2 is too large for ad-hoc TODO comments or a GitHub-only board. We need a
machine-readable, canonical record of what is implemented and how far, plus
automation that prevents the record from drifting away from the code.

## Scope

- Spec sections: none (tooling/docs).
- Crates/modules: `xtask/src/feature_status.rs`, `xtask/src/main.rs`.
- CLI/docs/tests: `cargo xtask feature-status` / `check-feature-status` /
  `spec-coverage`; `docs/IMPLEMENTATION-MATRIX.toml` (+ schema), `docs/FEATURE-TRACKING.md`,
  `docs/FEATURE-STATUS.md`, `docs/ENCODER-ROADMAP.md`, `docs/CONFORMANCE.md`,
  `docs/DECISIONS/0001-feature-tracking.md`, `openspec/`, GitHub templates.

## Non-goals

- No new AV2 codec syntax.
- No GitHub-as-source-of-truth; the matrix is canonical.
- No mandatory network or OpenSpec-CLI dependency in CI.

## Acceptance criteria

- [x] Implementation matrix exists and is canonical.
- [x] `cargo xtask feature-status` renders table/json/markdown.
- [x] `cargo xtask check-feature-status` validates the matrix and scans the tree.
- [x] `cargo xtask spec-coverage` summarizes coverage.
- [x] `check-feature-status` is wired into `cargo xtask ci` and CI.
- [x] Positive tests exist (`xtask/src/feature_status.rs::tests`).
- [x] `STATUS.md` is updated.
