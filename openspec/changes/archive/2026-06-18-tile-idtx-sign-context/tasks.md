## 1. Spec + matrix

- [x] 1.1 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` notes + proof in
  `docs/IMPLEMENTATION-MATRIX.toml` for `idtx_sign`; repoint `openspec_change`.
- [x] 1.2 Advance the `tile-cdf-selection-boundary` row in
  `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add `idtx_sign_ctx(quant_sign, level, row, col, txw)` and the
  `COEFF_BASE_RANGE` constant to `cdf/coeff_context.rs` (net the three QuantSign
  neighbours, map to a base context, then the Level-threshold raise).
- [x] 2.2 Add a module-level `const` spec-contract check (the non-test consumer).

## 3. Tests

- [x] 3.1 Tests for each `signc` bucket (5/6/1/2/0), the level-threshold raise
  (including the `== COEFF_BASE_RANGE` boundary and the zero-context no-raise), the
  missing-edge-neighbour skips, and short-slice / pathological-geometry totality;
  the minimal-fixture decode output stays unchanged (no-output-change).

## 4. Gate

- [x] 4.1 Regenerate the generated status docs.
- [x] 4.2 `cargo xtask ci` green.
