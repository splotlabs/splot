## Why

The coefficient-decode subsystem (which produces `Quant`) needs the §9.3
coefficient CDF banks wired into the `splot-decode` block CDF subset. `eob_extra`
is the smallest, fully self-contained first bank: its §8.3.2 selection is
context-free — "the cdf is given by `TileEobExtraCdf`" — so it needs no
`Level[]` buffers, scan order, eob position, or transform-block geometry. Wiring
it is additive plumbing toward the coefficient-decode loop, verifiable now and
unchanged in output.

## What Changes

- Advance Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` (stays partial) by
  adding the `eob_extra` coefficient CDF bank to the block CDF subset.
- In `crates/splot-decode/src/tile_payload/cdf/block_rows.rs`: add
  `EobExtraCdfRows = [[i32; CDF_ROW_LEN]; COEFF_CDF_Q_CONTEXTS]`, an `eob_extra`
  field on `BlockCdfRows` loaded from `splot-core`'s generated
  `DEFAULT_EOB_EXTRA_CDF`, a `BlockCdfSelector::EobExtra { coeff_cdf_q_ctx }`
  variant with `row` / `row_mut` arms (bounds-checked by the existing
  `checked_coeff_cdf_q_context`), the per-`coeff_cdf_q_ctx` iteration in
  `avg_from_tile` and `scale_counts_for_frame_end_update`, and a `#[cfg(test)]`
  accessor.
- In `crates/splot-decode/src/tile_payload/cdf.rs`: add
  `TileCdfArray::EobExtra`, `TileCdfSelector::EobExtra { coeff_cdf_q_ctx }`, the
  `row` / `row_mut` delegation, and a `#[cfg(test)]` accessor.
- Tests: the bank loads the generated defaults, the selector returns each
  `coeff_cdf_q_ctx` row and a typed `SelectorOutOfRange` at the bound, and a tile
  copy does not alias the frame.

Non-goals:

- No `coeffs()` decode-loop wiring (the bank is loaded but not read), so the
  minimal-fixture decode output is unchanged.
- No other coefficient CDF banks (eob_pt, dc_sign, coeff_base, coeff_br), no
  `Level[]` / scan / eob / geometry state, and no per-symbol context derivation.
- No reconstruction, hashes, Y4M, reference refresh, public API, AVM/dav2d
  invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the `eob_extra` coefficient CDF bank in the tile CDF
  selection subset, while broader §8.3 coefficient CDF selection and the
  coefficient decode loop remain partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/block_rows.rs`
- `crates/splot-decode/src/tile_payload/cdf.rs`
- `crates/splot-decode/src/tile_payload/cdf/tests.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
