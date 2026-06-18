## Why

The nonzero coefficient path can now initialize block state, read EOB syntax, and
walk caller-supplied scan positions, but it still cannot hand coefficient
base/base-range symbol reads to `SymbolDecoder` because those CDF rows are not in
the tile-local CDF subset. Exposing the row families is the next small,
verifiable boundary before reading base, base-EOB, and BR symbols.

## What Changes

- Add Feature ID `DECODE-COEFF-BASE-CDF-ROWS` for the decode-side coefficient
  base/base-EOB/base-range CDF row boundary.
- Add `crates/splot-decode/src/tile_payload/cdf/coeff_rows.rs`, wired through
  `block_rows.rs`, with generated §9.3 coefficient row storage and typed
  selectors for the ordinary non-IDTX families needed after the scan walk:
  `coeff_base`, `coeff_base_uv`, `coeff_base_lf`, `coeff_base_lf_uv`,
  `coeff_base_eob`, `coeff_base_eob_uv`, `coeff_base_lf_eob`,
  `coeff_base_lf_eob_uv`, `coeff_br`, `coeff_br_uv`, and `coeff_br_lf`.
- Extend row, row_mut, tile-copy, save/average, and frame-end count-scaling
  behavior for those banks with bounds-checked selector axes.
- Add focused tests proving generated-default loading, selector bounds errors,
  tile-copy non-aliasing, and row mutation through `read_block_symbol_trace`
  without consuming any new runtime decode path.
- Update the implementation matrix, decoder support matrix, decoder conformance
  coverage metadata, and generated status/coverage docs.

Non-goals:

- No coefficient symbol loop integration yet: the rows are loaded and selectable
  but not read by runtime `coeffs()`, so fixture output remains unchanged.
- No FSC, IDTX, parity-hidden-only, or sign-symbol row exposure beyond the
  ordinary non-IDTX base/base-EOB/base-range families listed above.
- No `get_scan` derivation, context derivation changes, nonzero `Level[]` /
  `Quant[]` writes, `read_quant`, dequantization, inverse transform, residual
  add, reconstruction, reference refresh, public API, AVM/dav2d invocation, or
  scheduler change.

## Capabilities

### New Capabilities

- `coeff-base-cdf-rows`: loaded-but-unread tile CDF rows for ordinary non-IDTX
  coefficient base, base-EOB, and base-range symbol families.

### Modified Capabilities

- `decoder-support`: records `DECODE-COEFF-BASE-CDF-ROWS` as a partial
  coefficient-decode support row while broader coefficient symbol consumption
  remains incomplete.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/block_rows.rs`
- `crates/splot-decode/src/tile_payload/cdf/coeff_rows.rs`
- `crates/splot-decode/src/tile_payload/cdf.rs`
- `crates/splot-decode/src/tile_payload/cdf/tests.rs`
- `crates/splot-decode/src/tile_payload/cdf/block_read.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
