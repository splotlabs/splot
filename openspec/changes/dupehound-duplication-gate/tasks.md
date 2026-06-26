# Tasks

## Matrix and docs

- [x] Add `INFRA-DUPEHOUND-DUPLICATION-GATE` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [x] Record the change in `openspec/changes/README.md` active table.

## Gate (Phase 0)

- [x] Add `tools/dupehound/budget.toml` with the measured ceiling and ratchet note.
- [x] Add `xtask check-duplication` (parse `dupehound scan --json` (default scope),
      compare deletable lines to the budget, run-if-present).
- [x] Wire `check-duplication` into `run_ci()`.
- [x] Install dupehound + run the budget gate and the PR-only `check --diff`
      ratchet in `.github/workflows/ci.yml`.
- [x] Document reuse-before-reimplement in `AGENTS.md` + `docs/agents/commands.md`.

## Dedup campaign — lower the production budget (ratchet, not zero)

- [ ] Collapse the 47× `into_bytes` / 23× `uvlc` header boilerplate; lower budget.
- [ ] Deduplicate the CDF `row` / `row_mut` accessors; lower budget.
- [ ] Work down the remaining production + test clusters, lowering the budget in
      each scoped, CI-green commit.

## Tests and proof

- [x] Add over/at/under-budget unit tests for the threshold logic.
- [x] Add proof commands to the matrix row.

## Checks

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --all-targets --locked`
- [ ] `cargo xtask check-feature-status`
- [ ] `cargo xtask ci`
