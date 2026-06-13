## 1. Runtime API

- [x] 1.1 Add `crates/splot-decode/src/limits.rs` with `DecodeOptions`, `DecodeLimits`, typed limit names, typed units, arithmetic operations, local limit errors, and result alias.
- [x] 1.2 Re-export the limits API from `crates/splot-decode/src/lib.rs` while preserving the existing unsupported diagnostic API and behavior.
- [x] 1.3 Keep `crates/splot-decode/Cargo.toml` and `Cargo.lock` unchanged.

## 2. Tests

- [x] 2.1 Add unit tests for stable limit names, units, threshold lookup, and field-specific helper routing.
- [x] 2.2 Add unit tests for inclusive checks: below/equal pass, above fails, zero rejects positive actuals, and unlimited accepts `u64::MAX`.
- [x] 2.3 Add unit tests for checked add/mul success and overflow error metadata.
- [x] 2.4 Add unit tests proving limit errors are local developer errors and the existing unsupported diagnostic descriptor remains unchanged.

## 3. Documentation And Status

- [x] 3.1 Update `docs/DECODER-ROADMAP.md` to describe the source-backed limits API and keep byte-consuming enforcement future.
- [x] 3.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` with a `decode-limits-runtime-api` row and keep `decode-limits-budget` partial.
- [x] 3.3 Add `DECODE-LIMITS-RUNTIME-API` to `docs/IMPLEMENTATION-MATRIX.toml` with proof commands and tests.
- [x] 3.4 Regenerate decoder support and feature status docs.

## 4. Verification And Review

- [x] 4.1 Run `openspec validate decode-limits-runtime-api --strict`.
- [x] 4.2 Run `cargo test -p splot-decode --locked`.
- [x] 4.3 Run `cargo clippy -p splot-decode --all-targets --locked -- -D warnings`.
- [x] 4.4 Run `cargo xtask check-diagnostic-registry`, `cargo xtask check-dependency-direction`, `cargo xtask check-decoder-support`, and `cargo xtask check-feature-status`.
- [x] 4.5 Run `cargo xtask ci`.
- [x] 4.6 Run required implementation, test, documentation, and final review subagents; record findings and fixes in `agent-log.md`.
