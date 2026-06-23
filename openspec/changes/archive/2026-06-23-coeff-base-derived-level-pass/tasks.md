## 1. Tracking And Docs

- [x] 1.1 Add Feature ID `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add the `coeff-base-derived-level-pass` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add decoder conformance coverage metadata for `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.
- [x] 1.4 Update `docs/DECODER-ROADMAP.md` to describe the loaded-but-unwired state-derived first pass.

## 2. Core Implementation

- [x] 2.1 Inspect the existing scan, base-symbol, level-state, coefficient-context, and TCQ helpers.
- [x] 2.2 Add a shared crate-private TCQ next-state helper for first-pass and second-pass use.
- [x] 2.3 Add selector mapping from derived `coeff_base_eob`, `coeff_base`, and `coeff_br` contexts to `CoeffCdfSelector`.
- [x] 2.4 Add a crate-private state-derived ordinary non-FSC base/level first-pass helper.
- [x] 2.5 Return decoded base reads, final `Level[]` block state, and first-pass `sumAbs1`/`numNz`/`isHidden`/`tcqState` summary.
- [x] 2.6 Keep runtime `coeffs()` integration and decode output unchanged.

## 3. Tests

- [x] 3.1 Cover successful state-derived first-pass composition producing local `Level[]` writes.
- [x] 3.2 Cover selector derivation from evolving `Level[]` by comparing against explicit selector rows.
- [x] 3.3 Cover TCQ selector-state updates before each base read and parity summary updates after level writes.
- [x] 3.4 Cover low-frequency chroma skipping `coeff_br` even when the base level exceeds the threshold.
- [x] 3.5 Cover a static preflight failure preserving CDF, symbol, and local state before reads.

## 4. Verification

- [x] 4.1 Regenerate `docs/FEATURE-STATUS.md`.
- [x] 4.2 Regenerate `docs/SPEC-COVERAGE.md`.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 4.4 Regenerate `docs/DECODER-SPEC-COVERAGE.md`.
- [x] 4.5 Run `openspec validate coeff-base-derived-level-pass --strict`.
- [x] 4.6 Run `cargo test -p splot-decode coeff_loop --locked`.
- [x] 4.7 Run `cargo xtask check-feature-status`.
- [x] 4.8 Run `cargo xtask check-decoder-support`.
- [x] 4.9 Run `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.10 Run `env RUSTUP_TOOLCHAIN=1.96.0-aarch64-apple-darwin cargo xtask ci`.
