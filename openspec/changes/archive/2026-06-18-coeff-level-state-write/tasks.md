## 1. Feature Metadata

- [x] 1.1 Add `DECODE-COEFF-LEVEL-STATE-WRITE` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add a `coeff-level-state-write` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-LEVEL-STATE-WRITE`.

## 2. Coefficient Level State Boundary

- [x] 2.1 Add a `coeff_loop/level_state.rs` module that applies decoded base-symbol levels to local `Level[]`.
- [x] 2.2 Wire the module through `coeff_loop.rs` without exceeding the source-line soft budget.
- [x] 2.3 Implement input-count, scan-entry, and block-coordinate preflight before any state writes.

## 3. Tests

- [x] 3.1 Add coverage proving row-major `Level[]` placement from decoded base/base-range reads.
- [x] 3.2 Add coverage proving `QuantSign[]` and `Quant[]` remain untouched.
- [x] 3.3 Add count/entry mismatch and mismatched block/walk geometry rejection tests.

## 4. Documentation and Verification

- [x] 4.1 Update `docs/DECODER-ROADMAP.md` and generated status/coverage docs.
- [x] 4.2 Run `cargo test -p splot-decode coeff_loop --locked`.
- [x] 4.3 Run `openspec validate coeff-level-state-write --strict`.
- [x] 4.4 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.5 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
