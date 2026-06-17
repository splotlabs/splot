## 1. Spec + matrix

- [x] 1.1 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` notes + proof in
  `docs/IMPLEMENTATION-MATRIX.toml` for `dc_sign`; repoint `openspec_change`.
- [x] 1.2 Advance the `tile-cdf-selection-boundary` row in
  `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add the `DcSignCdfRows` alias, the `dc_sign` field + default load, the
  `BlockCdfSelector::DcSign` variant with bounds-checked `row` / `row_mut` arms,
  `checked_dc_sign_group`, the parameterized `checked_plane_type`, and the
  flattened-iterator avg/scale folds in `block_rows.rs`.
- [x] 2.2 Add `TileCdfArray::DcSign`, `TileCdfSelector::DcSign`, and the
  delegation in `cdf.rs`.

## 3. Tests

- [x] 3.1 Default-load + full-index selection, four-axis out-of-range rejection,
  and tile-copy-no-alias tests; existing cdf tests + the minimal-fixture decode
  hash stay green (no-output-change).

## 4. Gate

- [x] 4.1 Regenerate the generated status docs.
- [x] 4.2 `cargo xtask ci` green.
