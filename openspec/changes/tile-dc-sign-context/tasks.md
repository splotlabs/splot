## 1. Spec + matrix

- [x] 1.1 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` notes + proof in
  `docs/IMPLEMENTATION-MATRIX.toml` for `dc_sign`; repoint `openspec_change`.
- [x] 1.2 Advance the `tile-cdf-selection-boundary` row in
  `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add `dc_sign_ctx(above_dc, left_dc, x4, y4, w4, h4)` to
  `cdf/coeff_context.rs` (netting above/left DC-sign votes, breaking once the
  monotonic index leaves the slice).
- [x] 2.2 Add a module-level `const` spec-contract check (the non-test consumer).

## 3. Tests

- [x] 3.1 Tests for the above/left netting (positive / negative / zero), the
  position offset, the out-of-slice (max-bound) skip, and pathological-geometry
  totality; the minimal-fixture decode output stays unchanged (no-output-change).

## 4. Gate

- [x] 4.1 Regenerate the generated status docs.
- [x] 4.2 `cargo xtask ci` green.
