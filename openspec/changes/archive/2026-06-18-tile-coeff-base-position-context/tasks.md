## 1. Spec + matrix

- [x] 1.1 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` notes + proof in
  `docs/IMPLEMENTATION-MATRIX.toml` for the coefficient base position contexts;
  repoint `openspec_change`.
- [x] 1.2 Advance the `tile-cdf-selection-boundary` row in
  `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add `cdf/coeff_context.rs` with `coeff_base_eob_ctx(c, bwl, height)`
  (the four `SIG_COEF_CONTEXTS_EOB` contexts, total over an out-of-range shift)
  and `coeff_base_bob_ctx(bob, seg_eob)` (contexts 0/1/2), both `const fn`.
- [x] 2.2 Register `pub(crate) mod coeff_context;` in `cdf.rs`.

## 3. Tests

- [x] 3.1 Boundary + neighbour tests for both contexts across TX_32X32 and TX_4X4
  geometry, a zero-`seg_eob` case, an out-of-range-shift totality case, and
  compile-time `const` checks; the minimal-fixture decode output stays unchanged
  (no-output-change).

## 4. Gate

- [x] 4.1 Regenerate the generated status docs.
- [x] 4.2 `cargo xtask ci` green.
