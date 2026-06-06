# Tasks

## Matrix and docs

- [x] Add `XTASK-CONVENTIONAL-COMMITS` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Regenerate `docs/FEATURE-STATUS.md`.
- [x] Document Conventional Commit title and subject requirements in `AGENTS.md`
      and `CONTRIBUTING.md`.
- [x] Add a PR checklist reminder.

## Implementation

- [x] Add `cargo xtask check-conventional-commits`.
- [x] Add `cargo xtask check-conventional-title`.
- [x] Wire the check into `.github/workflows/ci.yml`.

## Tests and proof

- [x] Add positive subject parser tests.
- [x] Add negative subject parser tests.
- [x] Add PR-title checker tests.
- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
