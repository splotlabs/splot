# Tasks

## Matrix and docs

- [x] Add `XTASK-DOC-BUDGET` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Add `docs/README.md` with retained-doc rationale and above-budget explanation.
- [x] Record the change in `openspec/changes/README.md`.

## Gate

- [x] Add `tools/docs/budget.toml`.
- [x] Add `cargo xtask check-doc-budget`.
- [x] Wire `check-doc-budget` into `cargo xtask ci`.
- [x] Run `cargo xtask check-doc-budget` in GitHub CI.

## Checks

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test -p xtask doc_budget --locked`
- [ ] `cargo xtask check-doc-budget`
- [ ] `cargo xtask check-feature-status`
- [ ] `cargo xtask ci`
