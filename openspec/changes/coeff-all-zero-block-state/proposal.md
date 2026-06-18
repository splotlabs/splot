## Why

The decoder now derives `all_zero` contexts from tile coefficient context state,
but the minimal trace does not yet apply the §5.20.7.27 all-zero coefficient
block effects back into that state. The next safe coefficient-loop step should
model the zero-output branch before nonzero EOB and coefficient entropy reads are
introduced.

## What Changes

- Add Feature ID `DECODE-COEFF-ALL-ZERO-BLOCK-STATE` for state-backed all-zero
  coefficient block effects.
- Add a new `coeff-all-zero-block-state` decoder-support row.
- Extend crate-private transform coefficient block state to carry the zeroed
  `Quant[]` buffer alongside `Level[]` and `QuantSign[]`.
- Add a crate-private `coeff_loop` helper that:
  - derives the adjusted coefficient extent from caller-resolved 4x4 transform
    dimensions;
  - initializes zero `Level[]`, `QuantSign[]`, and `Quant[]` state for an
    `all_zero == 1` block;
  - returns `eob == 0`, `culLevel == 0`, and `dcCategory == 0`;
  - applies the §5.20.7.27 above/left level/DC context writes through
    `TileCoeffContextState`.
- Wire the minimal flat-intra block-symbol trace to apply those all-zero state
  effects after the luma and V all-zero symbol reads.
- Keep decode output unchanged; the same symbols are read from the same CDF rows
  for the minimal fixture.

Non-goals:

- No nonzero EOB decode, coefficient scan walk, coefficient base/br/sign reads,
  `read_quant`, dequant, inverse-transform, residual add, reconstruction, output
  hash/raw/Y4M change, reference refresh, public API, AVM/dav2d invocation, or
  scheduler change.
- No U-plane `txb_skip` branch, broad transform-block geometry derivation,
  `TxTypes` writes, CCTX derivation, full `decode_block()`, full `decode_tile()`,
  inter prediction, loop filters, CDEF, loop-restoration, or film grain.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records that the minimal coefficient-loop frontier now
  applies the §5.20.7.27 all-zero block state effects while the full coefficient
  loop remains partial.

## Impact

- `crates/splot-decode/src/tile_payload/coeff_state.rs`
- `crates/splot-decode/src/tile_payload/coeff_loop.rs`
- `crates/splot-decode/src/tile_payload/block_symbol.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
