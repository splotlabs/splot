# Tasks

## Matrix and docs

- [x] Add `INFRA-CI-PIPELINE-SPEEDUPS` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Record the change in `openspec/changes/README.md`.
- [x] Add the `tooling` capability delta for the CI speed-up.
- [x] Update `README.md` and `docs/agents/commands.md` for the new gate shape.
- [x] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [x] Regenerate `docs/SPEC-COVERAGE.md` with `cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md`.

## Implementation

- [x] Remove the redundant explicit `cargo build --workspace --all-targets --locked`
      pass from `cargo xtask ci`.
- [x] Remove the same explicit build pass from `.github/workflows/ci.yml`.
- [x] Set `CARGO_INCREMENTAL=0` in GitHub CI and bump the cargo cache namespace.
- [x] Cache `~/.cargo/bin/dupehound` and skip `cargo install dupehound@0.1.2`
      on cache hits.

## Tests and proof

- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo test -p xtask --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
