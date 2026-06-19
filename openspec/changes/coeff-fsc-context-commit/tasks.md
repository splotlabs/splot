## 1. Core Implementation

- [x] 1.1 Add a crate-private FSC context-commit config and wrapper that runs
  the FSC quant pass and commits final `culLevel` / `dcCategory` through
  `TileCoeffContextState::update_after_coeffs`.

## 2. Tests

- [x] 2.1 Add focused FSC tests for successful above/left level and DC context
  writes, context preservation on pass failure, and context preservation on
  update failure.

## 3. Tracking And Validation

- [x] 3.1 Update `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, and
  `docs/DECODER-ROADMAP.md` for `DECODE-COEFF-FSC-CONTEXT-COMMIT`.
- [x] 3.2 Regenerate generated status docs and run OpenSpec, feature-status,
  decoder-support, decoder-conformance, focused tests, and full
  `cargo xtask ci` gates.
