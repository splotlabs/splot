## 1. Feature Metadata

- [x] 1.1 Add `DECODE-COEFF-QUANT-STATE-WRITE` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add a `coeff-quant-state-write` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-QUANT-STATE-WRITE`.

## 2. Coefficient Quant State Boundary

- [x] 2.1 Add a `coeff_loop/quant_state.rs` module for caller-provided `read_quant` outputs.
- [x] 2.2 Wire the module through `coeff_loop.rs` without exceeding the source-line soft budget.
- [x] 2.3 Implement count, scan-entry, sign-entry, coordinate, and `Quant[pos]` preflight before mutation.
- [x] 2.4 Implement hidden-parity, `culLevel`, `dcCategory`, optional TCQ, sign, and `Quant[pos]` state effects while preserving `QuantSign[]`.

## 3. Tests

- [x] 3.1 Add coverage for positive `Quant[pos]`, `culLevel`, and `dcCategory` writes.
- [x] 3.2 Add hidden-parity and optional TCQ adjustment coverage.
- [x] 3.3 Add coverage proving `QuantSign[]` remains unchanged.
- [x] 3.4 Add mismatch rejection tests that preserve local block state before mutation.

## 4. Documentation and Verification

- [x] 4.1 Update `docs/DECODER-ROADMAP.md` and generated status/coverage docs.
- [x] 4.2 Run `cargo test -p splot-decode coeff_loop --locked`.
- [x] 4.3 Run `openspec validate coeff-quant-state-write --strict`.
- [x] 4.4 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.5 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
