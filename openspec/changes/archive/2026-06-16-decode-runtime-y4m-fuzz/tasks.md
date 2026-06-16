## 1. Feature Tracking And OpenSpec

- [x] 1.1 Add `CONF-DECODE-RUNTIME-Y4M-FUZZ` to `docs/IMPLEMENTATION-MATRIX.toml` with scoped proof entries.
- [x] 1.2 Add `decode-runtime-y4m-fuzz` to `docs/DECODER-SUPPORT-MATRIX.toml` without broadening runtime decode or CLI filesystem publication claims.
- [x] 1.3 Run `openspec validate decode-runtime-y4m-fuzz --strict`.

## 2. Fuzz Target

- [x] 2.1 Add a `decode_runtime_y4m_bytes` bin to `fuzz/Cargo.toml`.
- [x] 2.2 Implement `fuzz/fuzz_targets/decode_runtime_y4m_bytes.rs` with bounded raw-byte and minimal-fixture mutation modes.
- [x] 2.3 Exercise successful in-memory Y4M output plus typed unsupported, malformed, resource-limit, and caller-writer output-error paths without filesystem output.

## 3. Documentation And Generated Status

- [x] 3.1 Update `docs/TESTING.md` and related fuzz target lists for the runtime Y4M fuzz target.
- [x] 3.2 Update decoder conformance coverage metadata for runtime Y4M fuzz coverage.
- [x] 3.3 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Review And Gates

- [x] 4.1 Run targeted gates: `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`, `cargo xtask check-fuzz-targets`, `cargo test -p splot-decode runtime_y4m --locked`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.2 Run a short local `cargo +nightly fuzz run decode_runtime_y4m_bytes` smoke when cargo-fuzz is available.
- [x] 4.3 Run independent review for fuzz target scope, writer behavior, resource bounds, no-panic behavior, and status honesty.
- [x] 4.4 Run `openspec validate --all --no-interactive` and `cargo xtask ci`.

## 5. Archive And PR

- [x] 5.1 Archive the OpenSpec change with `openspec archive decode-runtime-y4m-fuzz --yes` and commit the archive in this branch.
- [x] 5.2 Re-run relevant gates after archive.
- [x] 5.3 Open a ready, non-draft PR with Feature ID, scoped fuzz behavior, tests, reviewer decisions, and known exclusions.
