# Tasks

## Matrix and docs

- [x] Add the `XTASK-CONFLICT-ZONE-GUARD` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [x] Add `cargo xtask check-conflict-zone` to the `AGENTS.md` §4 command list.

## Implementation

- [x] Add `xtask/src/conflict_zone.rs`: a committed forbidden-path denylist, a
      pure `is_forbidden(path) -> Option<&'static str>` classifier, base
      resolution (`merge-base` vs `main`), and `check_conflict_zone(root)`.
- [x] Make it decoder-safe: skip-with-notice on no-base/empty-diff, on
      decoder-stream branches (a `decode`/`recon` name token, resolved from
      `SPLOT_PR_HEAD_REF` in CI), and when `SPLOT_SKIP_CONFLICT_ZONE=1`.
- [x] Wire it additively into `xtask/src/main.rs` (module, `Task::CheckConflictZone`,
      dispatch arm, and a `run_ci` step after `check-diagnostic-registry`).
- [x] Add a decoder-safe `.github/workflows/ci.yml` step.

## Tests and proof

- [x] Unit-test `is_forbidden` on representative decoder paths (true) and
      validator/shared paths (false), including `av2`-vs-`avm` and
      `openspec/changes/avm-*` false-positive guards.
- [x] Tokenized `is_decoder_branch` test incl. the `fix/reconcile-*`
      false-exemption guard; temp-git-repo integration test proving a
      rename-away/deletion of a decoder file is flagged (`--no-renames`).
- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-conflict-zone`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
