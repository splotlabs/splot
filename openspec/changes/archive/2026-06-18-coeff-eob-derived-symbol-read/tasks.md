## 1. OpenSpec and Feature Tracking

- [x] 1.1 Validate the `coeff-eob-derived-symbol-read` OpenSpec artifacts.
- [x] 1.2 Add `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.3 Add the corresponding decoder support and conformance-coverage rows.

## 2. Decode Helper

- [x] 2.1 Add a crate-private derived EOB symbol-read helper in `tile_payload/coeff_loop.rs`.
- [x] 2.2 Ensure invalid transform log2 inputs fail before CDF or symbol-decoder mutation.

## 3. Tests and Documentation

- [x] 3.1 Add focused tests for derived-read direct equivalence, invalid-input no-consumption, and propagated symbol-reader errors.
- [x] 3.2 Update decoder roadmap/status notes while preserving partial runtime support claims.
- [x] 3.3 Regenerate feature/status/support/coverage Markdown outputs.

## 4. Verification

- [x] 4.1 Run focused `splot-decode` coefficient-loop tests.
- [x] 4.2 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, decoder support, decoder conformance coverage, and OpenSpec validation.
- [x] 4.3 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
