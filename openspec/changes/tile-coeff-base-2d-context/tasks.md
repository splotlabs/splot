## 1. Spec + matrix

- [x] 1.1 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` notes + proof in
  `docs/IMPLEMENTATION-MATRIX.toml` for `coeff_base`; repoint `openspec_change`.
- [x] 1.2 Advance the `tile-cdf-selection-boundary` row in
  `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add `CoeffBaseContext` + `CoeffBaseSelection` + `select(&[u32])` in
  `cdf/coeff_context.rs`, using the generated `SIG_REF_DIFF_OFFSET` and the
  `SIG_REF_DIFF_OFFSET_NUM` / `LF_SIG_COEF_CONTEXTS_2D` /
  `LF_SIG_COEF_CONTEXTS_2D_UV` constants, with checked/saturating geometry.

## 3. Tests

- [x] 3.1 Tests for every branch (Hf 2D buckets, Hf non-2D, Lf 2D, Lf non-2D
  horiz/vert, chroma Uv/LfUv, the clamped neighbour sum, the magLimit raise, the
  parity-hidden override, the chroma `num`-distinguishing case, and short-slice /
  pathological-geometry totality); the minimal-fixture decode output stays
  unchanged (no-output-change).

## 4. Gate

- [x] 4.1 Regenerate the generated status docs.
- [x] 4.2 `cargo xtask ci` green.
