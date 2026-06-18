## Why

The decoder now owns tile-local coefficient context state, but the minimal
block-symbol frontier still supplies literal zero level/DC contributions to the
§8.3.2 `all_zero` (`txb_skip` / `v_txb_skip`) context formulas. The next
coefficient-loop step should read those contributions from the real
`TileCoeffContextState` lines before it starts consuming broader coefficient
syntax.

## What Changes

- Add Feature ID `DECODE-COEFF-ALL-ZERO-CONTEXT-STATE` for state-backed
  `all_zero` context derivation.
- Add a new `coeff-all-zero-context-state` decoder-support row.
- Add a crate-private `splot-decode` coefficient-loop foundation module that:
  - OR-reduces `AboveLevelContext` / `LeftLevelContext` over bounded transform
    ranges for luma `txb_skip`;
  - OR-reduces V-plane level and DC context lines for `v_txb_skip`;
  - feeds those reductions into the existing §8.3.2 context formula helpers;
  - is total over out-of-range starts and pathological caller counts by bounding
    iteration to the owned state slices.
- Wire the minimal flat-intra block-symbol trace to allocate
  `TileCoeffContextState` from the tile work-unit MI ranges and derive the
  existing luma/V `all_zero` contexts from state.
- Keep decode output unchanged; the same `txb_skip` / `v_txb_skip` symbols are
  read from the same CDF rows for the minimal fixture.

Non-goals:

- No EOB symbol decode, coefficient scan walk, `Quant[]`, `read_quant`,
  dequant, inverse-transform, residual add, reconstruction, output hash/raw/Y4M
  change, reference refresh, public API, AVM/dav2d invocation, or scheduler
  change.
- No U-plane `txb_skip` branch, broad transform-block geometry derivation,
  `EobU` derivation beyond the current minimal trace fact, full `decode_block()`,
  full `decode_tile()`, inter prediction, loop filters, CDEF, loop-restoration,
  or film grain.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records that the first `coeffs()`-adjacent context
  derivation now consumes real tile coefficient context state while the
  coefficient loop remains partial.

## Impact

- `crates/splot-decode/src/tile_payload/coeff_loop.rs`
- `crates/splot-decode/src/tile_payload/block_symbol.rs`
- `crates/splot-decode/src/tile_payload.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
