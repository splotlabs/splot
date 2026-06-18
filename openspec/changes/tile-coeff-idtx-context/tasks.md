## 1. Spec + matrix

- [x] 1.1 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` notes + proof in
  `docs/IMPLEMENTATION-MATRIX.toml` for the IDTX contexts; repoint `openspec_change`.
- [x] 1.2 Advance the `tile-cdf-selection-boundary` row in
  `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add `coeff_base_idtx_ctx`, `coeff_br_idtx_ctx`, and the shared
  `idtx_neighbour_mag` helper (saturating geometry + slice guard) in
  `cdf/coeff_context.rs`.
- [x] 2.2 Add a module-level `const` spec-contract check (the non-test consumer).

## 3. Tests

- [x] 3.1 Tests for the clamped left+above sum, the col==0 / row==0 skips, the br
  clamp-to-5-then-6 path, and short-slice / pathological-geometry totality; the
  minimal-fixture decode output stays unchanged (no-output-change).

## 4. Gate

- [x] 4.1 Regenerate the generated status docs.
- [x] 4.2 `cargo xtask ci` green.
