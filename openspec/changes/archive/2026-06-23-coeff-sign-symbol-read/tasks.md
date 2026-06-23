## 1. Feature Metadata

- [x] 1.1 Add `DECODE-COEFF-SIGN-SYMBOL-READ` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add a `coeff-sign-symbol-read` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-SIGN-SYMBOL-READ`.

## 2. Coefficient Sign Read Boundary

- [x] 2.1 Add a `coeff_loop/sign_symbol.rs` module for caller-resolved sign CDF/literal reads.
- [x] 2.2 Wire the module through `coeff_loop.rs` without exceeding the source-line soft budget.
- [x] 2.3 Implement input-count, scan-entry, level-coordinate, and required-sign preflight before reads.

## 3. Tests

- [x] 3.1 Add coverage for mixed `dc_sign`, `dc_sign_horz_vert`, `sign_bit`, and skipped zero-level entries.
- [x] 3.2 Add invalid CDF selector and input-count mismatch coverage.
- [x] 3.3 Add missing required sign and scan-entry mismatch rejection tests.

## 4. Documentation and Verification

- [x] 4.1 Update `docs/DECODER-ROADMAP.md` and generated status/coverage docs.
- [x] 4.2 Run `cargo test -p splot-decode coeff_loop --locked`.
- [x] 4.3 Run `openspec validate coeff-sign-symbol-read --strict`.
- [x] 4.4 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.5 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
