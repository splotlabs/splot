## Why

The nonzero coefficient path can now read EOB syntax, allocate local coefficient
state, walk caller-supplied scan positions, and select the ordinary non-IDTX
coefficient base/base-EOB/base-range CDF rows, but it still cannot consume those
rows as §5.20.7.27 symbols. Reading those symbols through a focused boundary is
the next small step before any nonzero `Level[]`, `Quant[]`, or reconstruction
mutation.

## What Changes

- Add Feature ID `DECODE-COEFF-BASE-SYMBOL-READ` for the ordinary non-FSC,
  non-IDTX coefficient base symbol-read boundary.
- Add a crate-private helper that accepts checked scan-walk entries plus
  caller-resolved `coeff_base_eob`, `coeff_base`, and `coeff_br` CDF selectors,
  reads the selected rows through `SymbolDecoder`, and returns the decoded
  level-building symbols in scan order.
- Keep selector validation transactional: invalid CDF selectors fail before
  symbol-decoder state is consumed or selected rows are updated.
- Add focused tests proving direct-read equivalence, disabled CDF update
  behavior, invalid-selector no-consumption behavior, and scan-entry ordering.
- Update the implementation matrix, decoder support matrix, decoder conformance
  coverage metadata, roadmap, and generated status/coverage docs.

Non-goals:

- No runtime `coeffs()` integration, no `get_scan` derivation, and no real
  transform-type or low-frequency derivation from block syntax.
- No FSC, IDTX, parity-hidden-only, sign-symbol, `dc_sign`, or `idtx_sign`
  reads.
- No nonzero `Level[]`, `QuantSign[]`, or `Quant[]` writes, no `read_quant`,
  dequantization, inverse transform, residual add, reconstruction, reference
  refresh, public API, AVM/dav2d invocation, dependency change, or scheduler
  change.

## Capabilities

### New Capabilities

- `coeff-base-symbol-read`: crate-private ordinary non-FSC coefficient
  base/base-EOB/base-range symbol-read boundary after EOB and checked scan-walk
  handoff.

### Modified Capabilities

- `decoder-support`: records `DECODE-COEFF-BASE-SYMBOL-READ` as a partial
  coefficient-decode support row while coefficient state writes and
  reconstruction remain incomplete.

## Impact

- `crates/splot-decode/src/tile_payload/coeff_loop.rs`
- `crates/splot-decode/src/tile_payload/coeff_loop/base_symbol.rs`
- `crates/splot-decode/src/tile_payload/coeff_loop/base_symbol_tests.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
