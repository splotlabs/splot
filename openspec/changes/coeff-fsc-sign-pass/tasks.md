## 1. FSC Sign Pass

- [x] 1.1 Add `NonZeroCoeffFscSignPass` and `apply_nonzero_coeff_fsc_sign_pass`.
- [x] 1.2 Add focused tests for skipped zero-level entries, `IdtxSign`
  selector derivation from evolving `QuantSign[]`, sign-to-`QuantSign[]`
  writes, untouched `Quant[]`, tx-size clamping, and static preflight
  no-consumption behavior.

## 2. Tracking And Verification

- [x] 2.1 Update `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, and
  `docs/DECODER-ROADMAP.md` for `DECODE-COEFF-FSC-SIGN-PASS`.
- [x] 2.2 Regenerate generated status documents and validate OpenSpec, feature
  status, decoder support, decoder conformance coverage, focused tests, and CI.
