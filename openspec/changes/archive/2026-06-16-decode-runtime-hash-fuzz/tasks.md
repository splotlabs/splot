## 1. Feature Tracking And OpenSpec

- [x] 1.1 Add `CONF-DECODE-RUNTIME-HASH-FUZZ` to `docs/IMPLEMENTATION-MATRIX.toml` with scoped proof entries.
- [x] 1.2 Add `decode-runtime-hash-fuzz` to `docs/DECODER-SUPPORT-MATRIX.toml` without broadening adjacent runtime decode rows.
- [x] 1.3 Run `openspec validate decode-runtime-hash-fuzz --strict`.

## 2. Fuzz Target

- [x] 2.1 Add `decode_runtime_hash_bytes` to `fuzz/Cargo.toml`.
- [x] 2.2 Implement `fuzz/fuzz_targets/decode_runtime_hash_bytes.rs` with raw arbitrary input mode, bounded fixture-mutation mode, and finite decode limits.
- [x] 2.3 Assert stable minimal hash-report structure on successful decode and accept typed `DecodeError` returns on failure.

## 3. Documentation And Generated Status

- [x] 3.1 Update `docs/TESTING.md` for the runtime hash fuzz target.
- [x] 3.2 Update decoder conformance coverage metadata for the new fuzz target.
- [x] 3.3 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Review And Gates

- [x] 4.1 Run targeted gates: `cargo check --manifest-path fuzz/Cargo.toml --bins`, `cargo xtask check-fuzz-targets`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.2 Re-run minimal runtime hash tests to prove output contract identity remains stable.
- [x] 4.3 Run independent subagent review for fuzz target scope, resource limits, and status honesty.
- [x] 4.4 Run `openspec validate --all --no-interactive` and `cargo xtask ci`.

## 5. Archive And PR

- [x] 5.1 Archive the OpenSpec change with `openspec archive decode-runtime-hash-fuzz --yes` and commit the archive in this branch.
- [x] 5.2 Re-run relevant gates after archive.
- [ ] 5.3 Open a ready, non-draft PR with Feature ID, scoped fuzz behavior, tests, reviewer decisions, and known exclusions.
