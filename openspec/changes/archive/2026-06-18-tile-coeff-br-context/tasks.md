## 1. Spec + matrix

- [x] 1.1 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` notes + proof in
  `docs/IMPLEMENTATION-MATRIX.toml` for `coeff_br`; repoint `openspec_change`.
- [x] 1.2 Advance the `tile-cdf-selection-boundary` row in
  `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add `CoeffBrContext` + `ctx(&[u32])` const fn, the
  `MAG_REF_OFFSET_WITH_TX_CLASS` table, and the `MAX_BASE_BR_RANGE` constant in
  `cdf/coeff_context.rs`; reuse `splot_recon::TransformClass`.
- [x] 2.2 Add a module-level `const` spec-contract check (the non-test consumer).

## 3. Tests

- [x] 3.1 Tests for the clamped-neighbour magnitude sum, the halve-and-clamp-to-6
  path, the plane/DC/LF `+7` and chroma `Min(mag, 3)` branches, the non-2D-chroma
  `num = 2` case (distinguished from `num = 3`), and out-of-bounds/short-slice
  totality; the minimal-fixture decode output stays unchanged (no-output-change).

## 4. Gate

- [x] 4.1 Regenerate the generated status docs.
- [x] 4.2 `cargo xtask ci` green.
