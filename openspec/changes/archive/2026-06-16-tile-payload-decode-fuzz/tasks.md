## 1. Feature Tracking And OpenSpec

- [x] 1.1 Add `CONF-TILE-PAYLOAD-DECODE-FUZZ` to `docs/IMPLEMENTATION-MATRIX.toml` with scoped proof entries.
- [x] 1.2 Add a decoder-support row for `tile-payload-decode-fuzz` and update `tile-payload-decode` proof without changing its partial status.
- [x] 1.3 Run `openspec validate tile-payload-decode-fuzz --strict`.

## 2. Fuzz Target

- [x] 2.1 Add a `tile_payload_decode_bytes` bin to `fuzz/Cargo.toml`.
- [x] 2.2 Add a feature-gated `splot-decode` fuzzing harness over the existing crate-private tile-payload boundary and minimal block-symbol frontier.
- [x] 2.3 Implement `fuzz/fuzz_targets/tile_payload_decode_bytes.rs` using bounded arbitrary tile payload bytes plus bounded known-good minimal-frontier payload mutations.
- [x] 2.4 Drive only the fuzzing harness with finite `DecodeLimits`, no filesystem output, no subprocesses, and no external decoder access.
- [x] 2.5 Assert only stable boundary/frontier success invariants and accept typed decode errors for malformed or unsupported mutations.

## 3. Documentation And Generated Status

- [x] 3.1 Update `docs/TESTING.md`, `AGENTS.md`, and CI fuzz-smoke comments/seeds for `tile_payload_decode_bytes`.
- [x] 3.2 Update decoder conformance coverage metadata for the tile-payload runtime fuzz evidence while keeping broad tile decode partial.
- [x] 3.3 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SUPPORT-STATUS.md`, and `docs/DECODER-SPEC-COVERAGE.md`.

## 4. Review And Gates

- [x] 4.1 Run targeted gates: `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`, `cargo xtask check-fuzz-targets`, `cargo test -p splot-decode runtime_hash --locked`, `cargo test -p splot-decode tile_payload --locked`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.2 Run a short local `cargo +nightly fuzz run tile_payload_decode_bytes` smoke when cargo-fuzz is available.
- [x] 4.3 Run independent review for scope honesty, public API usage, resource bounds, no-panic behavior, and matrix/status proof.
- [x] 4.4 Run `openspec validate --all --no-interactive` and `cargo xtask ci`.

## 5. Archive And PR

- [x] 5.1 Archive the OpenSpec change with `openspec archive tile-payload-decode-fuzz --yes` and commit the archive in this branch.
- [x] 5.2 Re-run relevant gates after archive.
- [ ] 5.3 Open a ready, non-draft PR with Feature ID, scoped fuzz behavior, tests, reviewer decisions, and known exclusions.
