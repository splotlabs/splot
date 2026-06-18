## 1. Decoder Implementation

- [x] 1.1 Add a crate-private derived-base ordinary-pass input/result/error path in `ordinary_pass.rs`.
- [x] 1.2 Delegate base selector derivation and `Level[]` writes to `apply_nonzero_coeff_base_derived_level_pass`.
- [x] 1.3 Feed first-pass `isHidden`, `sumAbs1`, and `useTcq` facts into the existing interleaved sign/quant composition.

## 2. Tests

- [x] 2.1 Add explicit-base versus derived-base equivalence coverage.
- [x] 2.2 Add hidden-parity first-pass summary handoff coverage.
- [x] 2.3 Add first-pass preflight failure coverage proving no symbol/CDF consumption.

## 3. Tracking And Gates

- [x] 3.1 Add `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS` rows to implementation, decoder-support, conformance coverage, and roadmap docs.
- [x] 3.2 Regenerate feature/spec/support/decoder-coverage status docs.
- [x] 3.3 Run focused tests, OpenSpec validation, feature/support checks, and `cargo xtask ci`.
