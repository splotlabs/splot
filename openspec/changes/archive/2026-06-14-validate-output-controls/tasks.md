# Tasks

## Matrix and docs

- [x] Add the `CLI-VALIDATE-OUTPUT-CONTROLS` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [x] Document the flags in the README quick-start.

## Implementation

- [x] Add `crates/splot-validate/src/render.rs`: `RenderOptions`, `RenderedReport`,
      `ReportSummary`, `Truncation`, and `render_text` / `rendered` on
      `ValidationReport`; re-export from `lib.rs`. Leave `Display` untouched.
- [x] Add `--max-diagnostics` / `--summary-only` to `ValidateArgs` and wire `run()`
      through the render API; leave the exit-code mapping unchanged.

## Tests and proof

- [x] Render unit tests incl. `render_text(default) == Display` parity, capping,
      summary-only, and `N == 0` (no panic).
- [x] `validate_snapshots.rs` golden snapshots (text + JSON) for both flags.
- [x] `cli.rs` behavioral tests: exit-code preservation, full counts under a cap,
      non-numeric `--max-diagnostics` rejected (exit 2).
- [x] Update the `validate --help` golden (intentional surface change).
- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
