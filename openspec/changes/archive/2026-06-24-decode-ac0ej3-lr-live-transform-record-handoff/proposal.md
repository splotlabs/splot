## Why

The ac0ej3 decoder mission currently has the pieces to derive a retained
`LrTxSkip` grid and to copy a complete retained grid into live LR storage, but
the live tile path still stops before handing parsed transform facts into that
storage. This change moves the fail-closed frontier from allocation-only
plumbing to a real tile-derived transform-record handoff.

Feature ID: `DECODE-AC0EJ3-LR-LIVE-TRANSFORM-RECORD-HANDOFF`.

## What Changes

- Collect luma transform skip/eob facts from the live key-tile residual path for
  the fixed-largest transform subset already modeled by the decoder.
- Convert those parsed transform facts into `WienerNsLrTxSkipTransformRecord`
  values and derive a complete `WienerNsLrTxSkipGrid` through the existing
  bounded helper.
- Populate the live LR storage shell from the tile-derived grid when fixed-
  largest transform records are available, then return the next fail-closed
  unsupported diagnostic for missing live frame samples.
- Move the local ac0ej3 mission stream from the allocation-only diagnostic to a
  precise selectable-transform-record frontier, because its key frame uses
  `TX_MODE_SELECT`.
- Keep decoded `CurrFrame`/`CdefFrame` samples, `FilterClass`,
  `SubclassLookup`, selectable transform partition parsing, loop-restoration
  filtering/output, reference refresh, AVM/dav2d equality, and successful
  ac0ej3 decode explicitly out of scope.

## Capabilities

### New Capabilities

- `ac0ej3-lr-live-transform-record-handoff`: Tracks the live runtime handoff
  from parsed luma tile transform facts to live LR `LrTxSkip` storage, plus the
  ac0ej3 fail-closed selectable-transform frontier.

### Modified Capabilities

- `decoder-support`: Adds a distinct partial support row for the new
  tile-derived transform-record handoff frontier.

## Impact

- Affected code is expected to stay inside `splot-decode` runtime/tile payload
  internals plus focused tests and the support/implementation matrices.
- No public decoder API, crate dependency graph, unsafe policy, encoder code, or
  third-party dependency changes are planned.
- Validator diagnostics remain structured and fail-closed; the local ac0ej3
  gate should advance only to the next honest unsupported runtime prerequisite.
