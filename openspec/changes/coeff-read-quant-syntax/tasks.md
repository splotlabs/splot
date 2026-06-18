## 1. Tracking And Artifacts

- [x] 1.1 Add Feature ID `DECODE-COEFF-READ-QUANT-SYNTAX` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `coeff-read-quant-syntax` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-READ-QUANT-SYNTAX`.
- [x] 1.4 Update `docs/DECODER-ROADMAP.md` to describe the loaded-but-unwired parser boundary.

## 2. Parser Implementation

- [x] 2.1 Inspect existing coefficient-loop helpers and literal-read wrappers.
- [x] 2.2 Add a crate-private § 5.20.7.28 `read_quant` syntax module with typed input, output, and errors.
- [x] 2.3 Implement threshold skip, finite q-length, Golomb extension, hidden DC `lvlShift`, TCQ doubling, and checked arithmetic.
- [x] 2.4 Keep runtime `coeffs()` integration and decode output unchanged.

## 3. Tests

- [x] 3.1 Add positive tests for threshold skip, finite q-length, Golomb extension, hidden DC, and TCQ paths.
- [x] 3.2 Add malformed-prefix and parser-error tests for q-length, Golomb-length, and coefficient-remainder paths.
- [x] 3.3 Add overflow/pathological-input tests proving typed errors and no panics.
- [x] 3.4 Run focused coefficient-loop tests.

## 4. Generated Status And Gates

- [x] 4.1 Regenerate `docs/FEATURE-STATUS.md`.
- [x] 4.2 Regenerate `docs/SPEC-COVERAGE.md`.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 4.4 Regenerate `docs/DECODER-SPEC-COVERAGE.md`.
- [x] 4.5 Run `openspec validate coeff-read-quant-syntax --strict`.
- [x] 4.6 Run `cargo xtask check-feature-status`.
- [x] 4.7 Run `cargo xtask check-decoder-support`.
- [x] 4.8 Run `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.9 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
