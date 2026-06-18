## 1. Decoder Implementation

- [x] 1.1 Add crate-private sign-source derivation config, error, and helper in `sign_symbol.rs`.
- [x] 1.2 Derive `dc_sign`, `dc_sign_horz_vert`, `sign_bit`, and skipped sign sources from local `Level[]`, hidden parity, plane, transform class, and DC contexts.

## 2. Tests

- [x] 2.1 Add luma DC `dc_sign` coverage, including hidden group and `dc_sign_ctx`.
- [x] 2.2 Add horizontal-axis and vertical-axis `dc_sign_horz_vert` coverage.
- [x] 2.3 Add generic `sign_bit`, chroma, zero-skip, hidden-parity, and state-error coverage.

## 3. Tracking And Gates

- [x] 3.1 Add `DECODE-COEFF-SIGN-SOURCE-DERIVE` rows to implementation, decoder-support, conformance coverage, and roadmap docs.
- [x] 3.2 Regenerate feature/spec/support/decoder-coverage status docs.
- [x] 3.3 Run focused tests, OpenSpec validation, feature/support checks, and `cargo xtask ci`.
