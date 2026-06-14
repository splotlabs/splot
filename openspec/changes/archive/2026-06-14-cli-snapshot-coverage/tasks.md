# Tasks

## Matrix and docs

- [x] Add the `CONF-CLI-SNAPSHOT-COVERAGE` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.

## Implementation

- [x] Add `crates/splot-cli/tests/help_snapshots.rs` snapshotting `validate --help`
      and `inspect --help`.
- [x] Add `crates/splot-cli/tests/inspect_text_snapshots.rs` snapshotting the
      `inspect` default and `--headers` text dump for representative fixtures.
- [x] Commit the deterministic `.snap` goldens under
      `crates/splot-cli/tests/snapshots/`.

## Tests and proof

- [x] Snapshots pass against committed goldens (no `INSTA_UPDATE`).
- [x] Goldens carry no paths/timestamps/version strings.
- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
