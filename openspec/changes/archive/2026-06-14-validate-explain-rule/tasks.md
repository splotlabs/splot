# Tasks

## Matrix and docs

- [x] Add the `CLI-VALIDATE-EXPLAIN` row to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [x] Document `splot explain` in the README quick-start. (`gen-explain`, like `gen-tables`, is a dev codegen command and is not listed in the AGENTS.md §4 user command set.)

## Implementation

- [x] Add `xtask/src/explain_registry.rs` (`cargo xtask gen-explain [--check]`)
      parsing the doc's 4-col tables; wire it into `main.rs` and `run_ci` (`--check`).
- [x] Add `crates/splot-validate/src/explain/` (`DiagnosticInfo`, `explain`/`all`/
      `did_you_mean`, generated `generated.rs`); re-export from `lib.rs`.
- [x] Add `crates/splot-cli/src/commands/explain.rs` + additive router wiring.

## Tests and proof

- [x] Codegen tests (parse/escape/grammar) + `gen-explain --check` drift gate.
- [x] `explain` lookup unit tests (sorted/unique, known/unknown, did-you-mean).
- [x] CLI describe text/JSON snapshots + behavioral tests (unknown/list/missing-arg)
      + `explain --help` golden.
- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask gen-explain --check`
- [x] `cargo xtask check-diagnostic-registry`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
