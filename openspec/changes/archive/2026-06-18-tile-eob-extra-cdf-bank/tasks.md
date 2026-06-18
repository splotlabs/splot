## 1. Spec + matrix

- [x] 1.1 Advance `DECODE-TILE-CDF-SELECTION-BOUNDARY` notes + proof in
  `docs/IMPLEMENTATION-MATRIX.toml` for the `eob_extra` bank; repoint
  `openspec_change`.
- [x] 1.2 Advance the `tile-cdf-selection-boundary` row in
  `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add the OpenSpec `decoder-support` ADDED delta for this change.

## 2. Implementation

- [x] 2.1 Add `EobExtraCdfRows`, the `eob_extra` field + default load, the
  `BlockCdfSelector::EobExtra` variant with `row` / `row_mut` arms, the
  avg/scale iteration, and the test accessor in `block_rows.rs`.
- [x] 2.2 Add `TileCdfArray::EobExtra`, `TileCdfSelector::EobExtra`, the
  delegation, and the test accessor in `cdf.rs`.

## 3. Tests

- [x] 3.1 Default-load assertion, selector + bounds-error test, and
  tile-copy-no-alias test; existing cdf tests + the minimal-fixture decode hash
  stay green (no-output-change).

## 4. Gate

- [x] 4.1 Regenerate the generated status docs.
- [x] 4.2 `cargo xtask ci` green.
