## 1. Decoder Implementation

- [x] 1.1 Update the derived-base ordinary-pass input/result path in `ordinary_pass.rs` to carry sign-source derivation facts instead of caller-supplied sign inputs.
- [x] 1.2 Derive sign inputs from the first-pass block state and summary after `apply_nonzero_coeff_base_derived_level_pass` succeeds.
- [x] 1.3 Feed the derived sign inputs into the existing interleaved sign, max-level, `read_quant`, and signed `Quant[]` composition without changing the explicit ordinary-pass API.

## 2. Tests

- [x] 2.1 Add explicit ordinary-pass versus derived-base/derived-sign equivalence coverage.
- [x] 2.2 Add hidden-parity coverage proving the final `c == 0` sign source is derived from the first-pass summary.
- [x] 2.3 Add invalid derived-sign selector coverage proving no quant syntax is consumed after the first pass.

## 3. Tracking And Gates

- [x] 3.1 Add `DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS` rows to implementation, decoder-support, conformance coverage, and roadmap docs.
- [x] 3.2 Regenerate feature/spec/support/decoder-coverage status docs.
- [x] 3.3 Run focused tests, OpenSpec validation, feature/support checks, and `cargo xtask ci`.
