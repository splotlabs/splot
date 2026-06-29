## Why

The workspace now enables `clippy::pedantic`, but several noisy lint families
remain allowed at workspace scope. Some of those exceptions are pragmatic for AV2
codec transcription, yet a global allow-list can grow into a permanent blind
spot unless changes to it are review-visible.

## What Changes

- Add Feature ID `XTASK-LINT-POLICY` to the implementation matrix.
- Add `cargo xtask check-lint-policy`.
- Wire the check into `cargo xtask ci`.
- Keep the existing Clippy allow-list working, but fail if a new
  workspace-level Clippy `allow` is added without an explicit xtask policy entry.
- Document that future tightening should remove, narrow, or replace existing
  global exceptions rather than expanding the list.

## Non-Goals

- Do not re-enable every currently allowed Clippy lint in this change.
- Do not mass-edit bitstream/parser modules to replace all casts or wildcard
  imports.
- Do not change the crate dependency graph or add dependencies.

## Acceptance Criteria

- [ ] `cargo xtask check-lint-policy` passes on the current workspace lint
  configuration.
- [ ] Focused `xtask` tests prove unknown workspace Clippy allows are rejected
  while removing existing debt allows is accepted.
- [ ] `cargo xtask ci` runs the lint-policy check.
