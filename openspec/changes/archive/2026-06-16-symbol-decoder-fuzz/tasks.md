## 1. Feature Tracking And OpenSpec

- [x] 1.1 Add `CONF-SYMBOL-DECODER-FUZZ` to `docs/IMPLEMENTATION-MATRIX.toml` with scoped proof entries.
- [x] 1.2 Add the `symbol-decoder-fuzz` row in `docs/DECODER-SUPPORT-MATRIX.toml` while keeping `symbol-decoder` partial.
- [x] 1.3 Run `openspec validate symbol-decoder-fuzz --strict`.

## 2. Fuzz Target

- [x] 2.1 Add a `symbol_decoder_bytes` bin to `fuzz/Cargo.toml`.
- [x] 2.2 Implement `fuzz/fuzz_targets/symbol_decoder_bytes.rs` with bounded payload, operation, and CDF-row inputs.
- [x] 2.3 Exercise `read_bool`, `read_literal`, `read_symbol`, and `exit_symbol` success and typed-error paths without filesystem or external decoder use.

## 3. Documentation And Generated Status

- [x] 3.1 Update `docs/TESTING.md`, `AGENTS.md`, and CI corpus comments/seeds for the symbol decoder fuzz target.
- [x] 3.2 Update decoder conformance coverage metadata for symbol decoder fuzz coverage.
- [x] 3.3 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Review And Gates

- [x] 4.1 Run targeted gates: `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`, `cargo xtask check-fuzz-targets`, `cargo test -p splot-core symbol --locked`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.2 Run a short local `cargo +nightly fuzz run symbol_decoder_bytes` smoke when cargo-fuzz is available.
- [x] 4.3 Run independent review for fuzz target scope, CDF construction, public API usage, resource bounds, no-panic behavior, and status honesty.
- [x] 4.4 Run `openspec validate --all --no-interactive` and `cargo xtask ci`.

## 5. Archive And PR

- [x] 5.1 Archive the OpenSpec change with `openspec archive symbol-decoder-fuzz --yes` and commit the archive in this branch.
- [x] 5.2 Re-run relevant gates after archive.
- [x] 5.3 Open a ready, non-draft PR with Feature ID, scoped fuzz behavior, tests, reviewer decisions, and known exclusions.
