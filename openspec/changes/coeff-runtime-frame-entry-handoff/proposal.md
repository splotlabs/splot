## Why

The minimal block-symbol runtime still calls the older ordinary all-zero
coefficient branch directly, even though the coefficient stack now has a higher
frame-facts entry wrapper. Routing the traced all-zero coefficient blocks through
that top wrapper lets runtime execution exercise the loaded `useFsc`/frame-facts
chain without changing decoded output or claiming broad nonzero `coeffs()`
support.

## What Changes

- Add Feature ID `DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF`.
- Route the minimal flat-intra luma and V all-zero coefficient applications
  through `apply_coeff_use_fsc_branch_from_frame_facts`.
- Preserve AV2 § 5.20.7.27 all-zero ordering: decoded all-zero blocks still
  bypass nonzero frame, segment, ordinary, FSC, parity, and TCQ fact derivation.
- Add focused tests proving the traced runtime output and CDF rollback behavior
  remain unchanged after entering the top coefficient wrapper.
- Update implementation matrix, decoder support matrix, roadmap, generated
  status docs, decoder conformance coverage metadata, and the audit ledger.
- Non-goals: runtime nonzero `coeffs()` wiring, full `compute_tx_type`, segment
  map derivation, transform-block syntax traversal, dequantization, inverse
  transform, residual add, reconstruction changes, output changes, reference
  refresh, encoder changes, dependency graph changes, and AVM/dav2d invocation.

## Capabilities

### New Capabilities

- `coeff-runtime-frame-entry-handoff`: minimal runtime coefficient all-zero
  execution enters the staged frame-facts coefficient wrapper.

### Modified Capabilities

- `decoder-support`: extend staged coefficient decode support with a partial row
  for runtime all-zero entry through the top coefficient frame-facts wrapper.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/block_symbol.rs`,
  focused block-symbol/runtime tests, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and decoder conformance coverage metadata.
- Public API impact: none; all touched helpers remain crate-private.
- Diagnostics impact: none; existing minimal runtime diagnostics and output
  bytes remain unchanged.
- Dependencies and licensing: no new dependencies and no licensing changes.
