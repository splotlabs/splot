# Tasks: diagnostic-registry-enforcement

## Matrix and docs

- [x] Add `XTASK-DIAGNOSTIC-REGISTRY` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.

## Implementation

- [x] Add `xtask/src/diagnostic_registry.rs`: comment-aware string scanner, `emitted_ids`
      (skip `#[cfg(test)]` modules and comments, exactly-one-slash grammar), `documented_ids`
      (marker region of the doc), `check_diagnostic_registry` (bidirectional set diff).
- [x] Reuse `is_diagnostic_id` / `collect_files` / `display_path` from `feature_status.rs`.
- [x] Wire `Task::CheckDiagnosticRegistry` subcommand and call it from `run_ci`.
- [x] Rewrite `docs/VALIDATOR-DIAGNOSTICS.md` with the marker-delimited authoritative tables
      (all emitted IDs incl. the `<ns>/syntax` registry-only sub-table); quarantine planning names.

## Tests and proof

- [x] Lexer tests: ignore IDs in `//`, `///`, nested `/* */`; honor escaped `\"`; `//`/`/*`
      inside a string are literal content.
- [x] Test-module exclusion tests; grammar rejection tests (trailing `-`, empty/2-slash/slash-less).
- [x] Marker-parser tests (in/out of markers, missing marker → error).
- [x] Check pass/fail tests on in-memory fixtures; resilient real-tree membership test.
- [x] Add proof commands to the matrix row.

## Co-evolution note

- [x] When a new diagnostic namespace is added, update BOTH `DIAGNOSTIC_PREFIXES`
      (`xtask/src/feature_status.rs`) and the registry tables in
      `docs/VALIDATOR-DIAGNOSTICS.md`; the two guards are complementary.
      (Standing guidance, not a one-off task: recorded durably in
      `docs/VALIDATOR-DIAGNOSTICS.md` and `docs/FEATURE-TRACKING.md`, so this
      change carries no open work.)

## Checks

- [x] `cargo xtask check-diagnostic-registry`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
