## 1. Tracking And Artifacts

- [x] 1.1 Add Feature ID `DECODE-COEFF-QUANT-PASS-COMPOSE` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `coeff-quant-pass-compose` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-QUANT-PASS-COMPOSE`.
- [x] 1.4 Update `docs/DECODER-ROADMAP.md` to describe the loaded-but-unwired composition boundary.

## 2. Implementation

- [x] 2.1 Inspect existing sign, `read_quant`, quant-state, and coefficient-state helpers.
- [x] 2.2 Add a crate-private ordinary non-FSC quant-pass composition module with typed input, output, and errors.
- [x] 2.3 Preflight counts, scan entries, local levels, hidden-parity sign presence and impossible TCQ/lossless pairings, max-level facts, and `Quant[]` positions before literal reads.
- [x] 2.4 Compose `read_nonzero_coeff_quants` with `apply_nonzero_coeff_quant_state`.
- [x] 2.5 Keep runtime `coeffs()` integration and decode output unchanged.

## 3. Tests

- [x] 3.1 Add a positive test proving `read_quant` outputs are fed into signed `Quant[]` writes.
- [x] 3.2 Add hidden-parity and TCQ tests proving one config drives both lower layers, rejects impossible pairings, and permits missing hidden DC sign syntax when `sumAbs1` is zero.
- [x] 3.3 Add no-consumption and no-mutation tests for bad caller facts before `read_quant`.
- [x] 3.4 Run focused coefficient-loop tests.

## 4. Generated Status And Gates

- [x] 4.1 Regenerate `docs/FEATURE-STATUS.md`.
- [x] 4.2 Regenerate `docs/SPEC-COVERAGE.md`.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 4.4 Regenerate `docs/DECODER-SPEC-COVERAGE.md`.
- [x] 4.5 Run `openspec validate coeff-quant-pass-compose --strict`.
- [x] 4.6 Run `cargo xtask check-feature-status`.
- [x] 4.7 Run `cargo xtask check-decoder-support`.
- [x] 4.8 Run `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.9 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
