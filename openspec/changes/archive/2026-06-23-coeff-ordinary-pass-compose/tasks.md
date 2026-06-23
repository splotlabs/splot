## 1. Tracking And Artifacts

- [x] 1.1 Add Feature ID `DECODE-COEFF-ORDINARY-PASS-COMPOSE` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `coeff-ordinary-pass-compose` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-ORDINARY-PASS-COMPOSE`.
- [x] 1.4 Update `docs/DECODER-ROADMAP.md` to describe the loaded-but-unwired ordinary pass composition.

## 2. Implementation

- [x] 2.1 Inspect the existing EOB, scan, base, level, sign, max-level, and quant-pass helpers.
- [x] 2.2 Add a crate-private ordinary non-FSC composition helper from nonzero block start through signed `Quant[]` writes.
- [x] 2.3 Keep runtime `coeffs()` integration and decode output unchanged.

## 3. Tests

- [x] 3.1 Cover successful ordinary pass composition producing local `Level[]` and signed `Quant[]`.
- [x] 3.2 Cover an early scan/base-input failure preserving CDF, symbol, and local state before later phases run.
- [x] 3.3 Cover a later sign or quant-pass failure returning a typed error without running subsequent phases.
- [x] 3.4 Cover spec-order interleaving of sign reads and `read_quant` consumption.

## 4. Generated Status And Gates

- [x] 4.1 Regenerate `docs/FEATURE-STATUS.md`.
- [x] 4.2 Regenerate `docs/SPEC-COVERAGE.md`.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 4.4 Regenerate `docs/DECODER-SPEC-COVERAGE.md`.
- [x] 4.5 Run `openspec validate coeff-ordinary-pass-compose --strict`.
- [x] 4.6 Run `cargo xtask check-feature-status`.
- [x] 4.7 Run `cargo xtask check-decoder-support`.
- [x] 4.8 Run `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.9 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
