# Tasks

## Matrix and docs

- [x] Add `docs/IMPLEMENTATION-MATRIX.toml` (+ schema, + row template).
- [x] Generate `docs/FEATURE-STATUS.md` from the matrix.
- [x] Add `docs/FEATURE-TRACKING.md`, `docs/ENCODER-ROADMAP.md`, `docs/CONFORMANCE.md`,
      `docs/DECISIONS/0001-feature-tracking.md`.
- [x] Add the `openspec/` structure and templates.
- [x] Update `README.md`, `AGENTS.md`, `STATUS.md`, `docs/SPEC-MAPPING.md`,
      `docs/TESTING.md`, `docs/CODE_REVIEW.md`, `.github/copilot-instructions.md`.

## Implementation

- [x] Typed matrix structs and loader in `xtask`.
- [x] `feature-status` (table/json/markdown, `--category`/`--kind`/`--output`).
- [x] `check-feature-status` (fields, ids, status values, proof, module paths,
      TODO scan, feature-token scan, diagnostic-prefix scan, status-doc drift).
- [x] `spec-coverage` (text/markdown).

## Tests and proof

- [x] Unit tests for matrix parsing and validation.
- [x] Proof commands recorded in the `XTASK-FEATURE-STATUS` row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
