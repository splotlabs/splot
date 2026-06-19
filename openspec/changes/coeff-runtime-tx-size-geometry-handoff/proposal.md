## Why

The minimal runtime now enters the top coefficient frame-facts wrapper for its
traced all-zero coefficient blocks, but it still passes raw `TX_*` enum ordinals
from local constants. Deriving those ordinals from the traced transform geometry
and generated AV2 section 9.2 `Tx_Width` / `Tx_Height` tables removes another
hard-coded runtime coefficient fact before nonzero `coeffs()` integration.

## What Changes

- Add Feature ID `DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF`.
- Replace the minimal block-symbol trace's local `TX_64X64` and `TX_16X16`
  constants with a checked geometry-to-`txSz` helper backed by generated AV2
  section 9.2 transform-size tables.
- Preserve the luma and V all-zero coefficient frame-entry path and all existing
  hash/raw/Y4M output bytes.
- Add focused tests proving traced 64x64 and 16x16 geometries resolve to the
  same wrapper inputs as before and reject unsupported geometry without CDF or
  symbol consumption.
- Update implementation matrix, decoder support matrix, roadmap, generated
  status/coverage docs, decoder conformance coverage metadata, and the audit
  ledger.
- Non-goals: nonzero runtime `coeffs()` wiring, transform-block syntax
  traversal, `compute_tx_type`, segment-map derivation, dequantization, inverse
  transform, residual add, reconstruction changes, output changes, reference
  refresh, encoder changes, dependency graph changes, and AVM/dav2d invocation.

## Capabilities

### New Capabilities

- `coeff-runtime-tx-size-geometry-handoff`: minimal runtime all-zero coefficient
  frame-entry inputs derive `txSz` from traced transform geometry and generated
  AV2 transform-size tables.

### Modified Capabilities

- `decoder-support`: extend staged coefficient decode support with a partial row
  for runtime all-zero transform geometry to `txSz` handoff.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/block_symbol.rs`,
  focused block-symbol/runtime tests, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and decoder conformance coverage metadata.
- Public API impact: none; all touched helpers remain crate-private.
- Diagnostics impact: none; existing minimal runtime diagnostics and output
  bytes remain unchanged.
- Dependencies and licensing: no new dependencies and no licensing changes.
