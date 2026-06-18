## 1. Spec + matrix

- [x] 1.1 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` notes + proof in
  `docs/IMPLEMENTATION-MATRIX.toml` for the `eob_pt` family; repoint
  `openspec_change`.
- [x] 1.2 Advance the `tile-cdf-selection-boundary` row in
  `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add the seven `EobPt*CdfRows` aliases, the `EobPtSize` enum, the seven
  fields + default loads, the `BlockCdfSelector::EobPt` variant with `row` /
  `row_mut` arms, `checked_eob_plane_ctx`, and the generic avg/scale helpers in
  `block_rows.rs`.
- [x] 2.2 Add `TileCdfArray::EobPt`, `TileCdfSelector::EobPt`, and the delegation
  in `cdf.rs`.

## 3. Tests

- [x] 3.1 Per-size default-load + selection, out-of-range `coeff_cdf_q_ctx` /
  `eob_ctx` rejection, and tile-copy-no-alias tests; existing cdf tests + the
  minimal-fixture decode hash stay green (no-output-change).

## 4. Gate

- [x] 4.1 Regenerate the generated status docs.
- [x] 4.2 `cargo xtask ci` green.
