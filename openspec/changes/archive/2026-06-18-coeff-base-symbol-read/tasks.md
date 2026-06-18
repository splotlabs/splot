## 1. Feature Metadata

- [x] 1.1 Add `DECODE-COEFF-BASE-SYMBOL-READ` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add a `coeff-base-symbol-read` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-BASE-SYMBOL-READ`.

## 2. Coefficient Base Symbol Boundary

- [x] 2.1 Add a `coeff_loop/base_symbol.rs` module with caller-resolved base/base-EOB and conditional base-range read inputs.
- [x] 2.2 Wire the module through `coeff_loop.rs` without growing the root file beyond the soft source-line budget.
- [x] 2.3 Implement checked scan-entry/input matching, direct base symbol reads, conditional BR reads, and decoded level summaries.

## 3. Tests

- [x] 3.1 Add direct-read equivalence coverage for base-EOB, base, and BR rows.
- [x] 3.2 Add tests for scan-entry mismatch and invalid selector no-consumption behavior.
- [x] 3.3 Add tests for disabled CDF update behavior and unreachable conditional BR selectors.

## 4. Documentation and Verification

- [x] 4.1 Update `docs/DECODER-ROADMAP.md` and generated status/coverage docs.
- [x] 4.2 Run `cargo test -p splot-decode coeff_loop --locked`.
- [x] 4.3 Run `openspec validate coeff-base-symbol-read --strict`.
- [x] 4.4 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.5 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
