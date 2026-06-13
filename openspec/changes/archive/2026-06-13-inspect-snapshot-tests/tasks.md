# Tasks

## Matrix and docs

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` `CONF-INSPECT-SNAPSHOTS` (`tests=done`,
      `mapped=done`, notes, proof block).
- [x] Regenerate `docs/FEATURE-STATUS.md` / `docs/SPEC-COVERAGE.md` if affected.

## Implementation

- [x] Add `insta` as a dev-only workspace dependency; reference it in `splot-cli` dev-deps.
- [x] Add `crates/splot-cli/tests/inspect_snapshots.rs` running `splot inspect --json` over a
      diverse set of committed fixtures and asserting golden snapshots.
- [x] Generate and commit the `.snap` files in `crates/splot-cli/tests/snapshots/`.

## Tests and proof

- [x] The snapshots pass deterministically on a clean re-run.
- [x] Record the test + command + fixtures in the matrix proof block.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo deny check bans licenses sources`
- [x] `cargo xtask ci`
