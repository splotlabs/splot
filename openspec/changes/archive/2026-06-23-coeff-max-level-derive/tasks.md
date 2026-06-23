## 1. Tracking And Artifacts

- [x] 1.1 Add Feature ID `DECODE-COEFF-MAX-LEVEL-DERIVE` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `coeff-max-level-derive` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-MAX-LEVEL-DERIVE`.
- [x] 1.4 Update `docs/DECODER-ROADMAP.md` to describe the loaded-but-unwired derivation boundary.

## 2. Implementation

- [x] 2.1 Inspect the existing scan-walk and quant-pass helpers.
- [x] 2.2 Add a crate-private ordinary non-FSC `maxLevel` derivation module with typed transform-class input.
- [x] 2.3 Provide conversion into existing quant-pass input records.
- [x] 2.4 Keep runtime `coeffs()` integration and decode output unchanged.

## 3. Tests

- [x] 3.1 Cover luma/chroma low-frequency limits for 2D, horizontal, and vertical transform classes.
- [x] 3.2 Cover hidden final-scan-entry override and non-final hidden behavior.
- [x] 3.3 Cover quant-pass input conversion and pathological coordinate totality.

## 4. Generated Status And Gates

- [x] 4.1 Regenerate `docs/FEATURE-STATUS.md`.
- [x] 4.2 Regenerate `docs/SPEC-COVERAGE.md`.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 4.4 Regenerate `docs/DECODER-SPEC-COVERAGE.md`.
- [x] 4.5 Run `openspec validate coeff-max-level-derive --strict`.
- [x] 4.6 Run `cargo xtask check-feature-status`.
- [x] 4.7 Run `cargo xtask check-decoder-support`.
- [x] 4.8 Run `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.9 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
