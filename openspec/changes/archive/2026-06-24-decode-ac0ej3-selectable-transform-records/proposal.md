## Why

The ac0ej3 mission stream now reaches the live LR transform-record handoff but
stops because the key frame uses `TX_MODE_SELECT`. The next decoder slice must
parse the §5.20.6.1 selectable transform-size/partition records needed to
derive real luma `LrTxSkip` values instead of reporting that frontier.

Feature ID: `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`.

## What Changes

- Add decoder-private selectable transform-record derivation for the supported
  ac0ej3 intra LR path, grounded in AV2 §5.20.6.1/§5.20.6.3
  `read_tx_size` and `read_tx_partition`.
- Feed the derived per-transform luma extents and coefficient `eob` facts into
  the existing `WienerNsLrTxSkipTransformRecord` and live `LrTxSkip` storage
  handoff.
- Move the local ac0ej3 diagnostic past
  `unsupported_wienerns_lr_tx_mode_select_transform_records` to the next honest
  unsupported runtime prerequisite after live `LrTxSkip` population.
- Keep decoded `CurrFrame`/`CdefFrame` sample population, `FilterClass`
  retention, `SubclassLookup`, loop-restoration filtering/output, reference
  refresh, AVM/dav2d equality, and successful ac0ej3 decode explicitly out of
  scope.

## Capabilities

### New Capabilities

- `ac0ej3-selectable-transform-records`: Tracks the ac0ej3 runtime capability
  to parse supported `TX_MODE_SELECT` transform-size/partition syntax and
  produce luma transform records for LR `LrTxSkip` derivation.

### Modified Capabilities

- `ac0ej3-lr-live-transform-record-handoff`: Changes the selectable-transform
  scenario from fail-closed at `TX_MODE_SELECT` to live `LrTxSkip` population
  when the supported selectable records are parsed.
- `decoder-support`: Adds a distinct partial support row for the selectable
  transform-record runtime frontier.

## Impact

- Affected code is expected to stay inside `splot-decode` runtime/tile-payload
  internals plus focused tests and the support/implementation matrices.
- No public decoder API, crate dependency graph, unsafe policy, encoder code, or
  third-party dependency changes are planned.
- Diagnostics remain structured and fail-closed; the ac0ej3 local stream should
  advance only to the next unsupported decoder prerequisite after transform
  records have populated live `LrTxSkip`.
