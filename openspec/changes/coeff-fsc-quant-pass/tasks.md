## 1. Core Implementation

- [x] 1.1 Expose the existing quant-state accumulator step needed by FSC without duplicating signed `Quant[]` arithmetic.
- [x] 1.2 Add a crate-private FSC sign/quant pass that consumes `NonZeroCoeffFscLevelPass`, interleaves `idtx_sign` and `read_quant` with FSC constants, writes `QuantSign[]` and signed `Quant[]`, and returns final local block state.

## 2. Tests

- [x] 2.1 Add focused FSC quant-pass tests for successful quant reads/writes, zero-level entries, `QuantSign[]` preservation, interleaved sign/quant ordering, `culLevel`, `dcCategory`, and fail-atomic static validation.

## 3. Tracking And Validation

- [x] 3.1 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, `xtask/src/decoder_conformance_coverage.rs`, and `docs/DECODER-ROADMAP.md` for `DECODE-COEFF-FSC-QUANT-PASS`.
- [x] 3.2 Regenerate generated status docs and run OpenSpec, feature-status, decoder-support, decoder-conformance, focused tests, and full `cargo xtask ci` gates.
