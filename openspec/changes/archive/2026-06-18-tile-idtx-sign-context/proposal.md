## Why

`idtx_sign` is the last §8.3.2 coefficient-symbol CDF context. With it the entire
§8.3.2 coefficient-context layer (eob/bob position, coeff_br, the IDTX magnitude
variants, coeff_base, and both sign contexts) is derived. It is verifiable now
over caller-provided `QuantSign[]` and `Level[]` slices.

## What Changes

- Advance Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` (stays partial) by
  adding the `idtx_sign` sign context.
- In `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs` add
  `idtx_sign_ctx(quant_sign, level, row, col, txw) -> usize` implementing §8.3.2
  `idtx_sign`: net the signs of the left (`QuantSign[row*txw + col-1]`), above
  (`QuantSign[(row-1)*txw + col]`), and above-left (`QuantSign[(row-1)*txw +
  col-1]`) coefficients into `signc`; map it to a base context (`5` for `signc >
  2`, `6` for `signc < -2`, `1` for `signc > 0`, `2` for `signc < 0`, else `0`);
  then add `2` when `Level[row][col] > COEFF_BASE_RANGE` and the base context is
  non-zero.
- Add the `COEFF_BASE_RANGE` constant.

The edge neighbours are gated by `col > 0` / `row > 0`; the flat index is
saturating and slice-bounds-guarded, so the `const fn` is total and panic-free. A
module-level `const` spec-contract check is the non-test consumer.

Non-goals:

- No `coeffs()` decode-loop wiring and no `QuantSign[]` / `Level[]` buffer
  construction (the derivation is not read by any decode path), so the
  minimal-fixture decode output is unchanged.
- No per-transform-block level/sign tile buffers.
- No reconstruction, hashes, Y4M, reference refresh, public API, AVM/dav2d
  invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the `idtx_sign` sign context — the last §8.3.2
  coefficient-symbol context — in the tile CDF selection subset, while the
  coefficient decode loop and its tile buffers remain partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
