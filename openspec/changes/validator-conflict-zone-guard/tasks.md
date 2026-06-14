# Tasks

## Matrix and docs

- [ ] Add the `XTASK-CONFLICT-ZONE-GUARD` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [ ] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [ ] Add `cargo xtask check-conflict-zone` to the `AGENTS.md` §4 command list.

## Implementation

- [ ] Add `xtask/src/conflict_zone.rs`: a committed forbidden-path denylist, a
      pure `is_forbidden(path) -> Option<&'static str>` classifier, base
      resolution (`merge-base` vs `main`), and `check_conflict_zone(root)`.
- [ ] Make it decoder-safe: skip-with-notice on no-base/empty-diff, on
      decoder-stream branches (name contains `decode`/`recon`), and when
      `SPLOT_SKIP_CONFLICT_ZONE=1`.
- [ ] Wire it additively into `xtask/src/main.rs` (module, `Task::CheckConflictZone`,
      dispatch arm, and a `run_ci` step after `check-diagnostic-registry`).
- [ ] Add a decoder-safe `.github/workflows/ci.yml` step.

## Tests and proof

- [ ] Unit-test `is_forbidden` on representative decoder paths (true) and
      validator/shared paths (false), including `av2`-vs-`avm` and
      `openspec/changes/avm-*` false-positive guards.
- [ ] Add proof commands to the matrix row.

## Checks

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --all-targets --locked`
- [ ] `cargo xtask check-conflict-zone`
- [ ] `cargo xtask check-feature-status`
- [ ] `cargo xtask ci`
