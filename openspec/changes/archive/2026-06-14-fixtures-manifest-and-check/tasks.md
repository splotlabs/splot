# Tasks

## Matrix and docs

- [x] Add the `XTASK-CHECK-FIXTURES` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [x] Author `docs/FIXTURES.md`; add `cargo xtask check-fixtures` to the `AGENTS.md` §4 command list.

## Implementation

- [x] Author `tests/fixtures/MANIFEST.toml` (one `[[fixture]]` per committed `.av2`).
- [x] Add `xtask/src/fixtures.rs`: hermetic hash/presence/orphan/uniqueness +
      category/expect consistency check, wired into `main.rs` and `run_ci`.
- [x] Add the `.github/workflows/ci.yml` `check-fixtures` step.
- [x] Add `crates/splot-cli/tests/fixture_manifest.rs` verifying each `expect`
      against `splot_validate::Validator` in-process.

## Tests and proof

- [x] Unit tests for sha256 format + category/expect consistency + manifest parse.
- [x] `cargo xtask check-fixtures` passes against the committed corpus.
- [x] The in-process outcome test passes (anti-vacuity + orphan guard).
- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-fixtures`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
