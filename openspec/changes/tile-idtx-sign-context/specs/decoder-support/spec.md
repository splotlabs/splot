## ADDED Requirements

### Requirement: idtx_sign sign CDF context

The `splot-decode` tile CDF selection subset SHALL derive the AV2 § 8.3.2 `idtx_sign` sign CDF context, tracked by `DECODE-TILE-CDF-SELECTION-BOUNDARY`. `idtx_sign_ctx` SHALL net the signs of the left (`QuantSign[row*txw + col-1]`), above (`QuantSign[(row-1)*txw + col]`), and above-left (`QuantSign[(row-1)*txw + col-1]`) coefficients into `signc` — the edge neighbours gated by `col > 0` and `row > 0` — map `signc` to a base context (`5` when `signc > 2`, `6` when `signc < -2`, `1` when `signc > 0`, `2` when `signc < 0`, else `0`), and add `2` when `Level[row][col]` exceeds `COEFF_BASE_RANGE` and the base context is non-zero (the inner index of `TileIdtxSignCdf[Min(TX_16X16, txSzCtx)][ctx]`). It SHALL read caller-provided row-major `txw`-wide `QuantSign[]` and `Level[]` slices with saturating flat-index geometry and a slice-length guard, so out-of-range reads contribute `0` and the function is total and panic-free. It SHALL NOT be read by any decode path in this change, so the minimal-fixture decode output SHALL be unchanged. The per-transform-block level/sign tile buffers and the coefficient decode loop remain partial.

#### Scenario: idtx_sign maps the neighbour sign sum and level threshold

- **WHEN** `cargo test -p splot-decode tile_payload --locked` runs
- **THEN** `idtx_sign_ctx` returns the base context for each `signc` bucket
  (5/6/1/2/0) and adds 2 only when the base context is non-zero and the current
  level exceeds `COEFF_BASE_RANGE`, with tests pinning the missing-edge-neighbour
  skips and the threshold boundary
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: idtx_sign is total over short slices and bad geometry

- **WHEN** the `QuantSign[]` / `Level[]` slices are shorter than the block or the
  geometry is malformed
- **THEN** out-of-range reads contribute `0` and `idtx_sign_ctx` returns without
  panicking
- **AND** library code does not panic, overflow, or unwrap

#### Scenario: Adding the context does not change decode output

- **WHEN** the minimal flat-intra fixture is decoded to a hash, raw, or Y4M output
- **THEN** the output bytes are identical to before the context was added (the
  derivation is not read by any decode path)

#### Scenario: Coefficient CDF selection remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the tile CDF selection boundary records the `idtx_sign` sign context
  (completing the § 8.3.2 coefficient-symbol contexts)
- **AND** broader coefficient decode (the per-transform-block level/sign tile
  buffers and the `coeffs()` loop) remains partial
