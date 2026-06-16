## 1. Feature Tracking And OpenSpec

- [x] 1.1 Add `CONF-RECON-Y4M-OUTPUT-FUZZ` to `docs/IMPLEMENTATION-MATRIX.toml` with scoped proof entries.
- [x] 1.2 Add `recon-y4m-output-fuzz` to `docs/DECODER-SUPPORT-MATRIX.toml` without broadening runtime decode or output claims.
- [x] 1.3 Run `openspec validate recon-y4m-output-fuzz --strict`.

## 2. Fuzz Target

- [x] 2.1 Add a direct `splot-recon` path dependency and `recon_y4m_output_bytes` bin to `fuzz/Cargo.toml`.
- [x] 2.2 Implement `fuzz/fuzz_targets/recon_y4m_output_bytes.rs` with bounded structured frame generation for supported Y4M formats.
- [x] 2.3 Exercise successful matching-frame serialization plus typed mismatch or writer-error paths without filesystem output.

## 3. Documentation And Generated Status

- [x] 3.1 Update `docs/TESTING.md` and related fuzz target lists for the Y4M serialization fuzz target.
- [x] 3.2 Update decoder conformance coverage metadata for Y4M serialization fuzz coverage.
- [x] 3.3 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Review And Gates

- [x] 4.1 Run targeted gates: `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`, `cargo xtask check-fuzz-targets`, `cargo test -p splot-recon y4m --locked`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.2 Run a short local `cargo +nightly fuzz run recon_y4m_output_bytes` smoke when cargo-fuzz is available.
- [x] 4.3 Run independent review for fuzz target scope, resource bounds, and status honesty.
- [x] 4.4 Run `openspec validate --all --no-interactive` and `cargo xtask ci`.

## 5. Archive And PR

- [x] 5.1 Archive the OpenSpec change with `openspec archive recon-y4m-output-fuzz --yes` and commit the archive in this branch.
- [x] 5.2 Re-run relevant gates after archive.
- [x] 5.3 Open a ready, non-draft PR with Feature ID, scoped fuzz behavior, tests, reviewer decisions, and known exclusions.
