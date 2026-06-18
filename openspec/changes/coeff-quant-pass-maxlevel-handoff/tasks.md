## 1. Tracking And Artifacts

- [x] 1.1 Add Feature ID `DECODE-COEFF-QUANT-PASS-MAXLEVEL-HANDOFF` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `coeff-quant-pass-maxlevel-handoff` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-QUANT-PASS-MAXLEVEL-HANDOFF`.
- [x] 1.4 Update `docs/DECODER-ROADMAP.md` to describe the loaded-but-unwired handoff.

## 2. Implementation

- [x] 2.1 Inspect the existing max-level derivation and quant-pass composer.
- [x] 2.2 Add a crate-private quant-pass wrapper that derives max-level inputs.
- [x] 2.3 Keep runtime `coeffs()` integration and decode output unchanged.

## 3. Tests

- [x] 3.1 Cover luma low-frequency derived max-level inputs reaching below-threshold `read_quant`.
- [x] 3.2 Cover hidden final-entry max-level override reaching the extended `read_quant` path.
- [x] 3.3 Cover bad quant-pass facts failing without consuming symbols or mutating state.

## 4. Generated Status And Gates

- [x] 4.1 Regenerate `docs/FEATURE-STATUS.md`.
- [x] 4.2 Regenerate `docs/SPEC-COVERAGE.md`.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 4.4 Regenerate `docs/DECODER-SPEC-COVERAGE.md`.
- [x] 4.5 Run `openspec validate coeff-quant-pass-maxlevel-handoff --strict`.
- [x] 4.6 Run `cargo xtask check-feature-status`.
- [x] 4.7 Run `cargo xtask check-decoder-support`.
- [x] 4.8 Run `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.9 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
