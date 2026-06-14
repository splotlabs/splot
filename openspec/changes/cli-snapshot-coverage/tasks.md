# Tasks

## Matrix and docs

- [ ] Add the `CONF-CLI-SNAPSHOT-COVERAGE` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [ ] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.

## Implementation

- [ ] Add `crates/splot-cli/tests/help_snapshots.rs` snapshotting `validate --help`
      and `inspect --help`.
- [ ] Add `crates/splot-cli/tests/inspect_text_snapshots.rs` snapshotting the
      `inspect` default and `--headers` text dump for representative fixtures.
- [ ] Commit the deterministic `.snap` goldens under
      `crates/splot-cli/tests/snapshots/`.

## Tests and proof

- [ ] Snapshots pass against committed goldens (no `INSTA_UPDATE`).
- [ ] Goldens carry no paths/timestamps/version strings.
- [ ] Add proof commands to the matrix row.

## Checks

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --all-targets --locked`
- [ ] `cargo xtask check-feature-status`
- [ ] `cargo xtask ci`
