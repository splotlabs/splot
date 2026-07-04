## Why

The local decoder mission stream now stops at the §7.20.4 pixel-classified Wiener gate
before the runtime can prove the source-read dependencies that precede Wiener NS
filtering. The next honest brick is to enumerate those classified-luma
dependencies and still fail closed before reading frame-buffer values, deriving
classes, or applying loop restoration.

## What Changes

- Add Feature ID `DECODE-LR-CLASSIFIED-WIENER-FRONTIER` for the narrow
  classified-Wiener dependency frontier after the existing LR source-read row.
- Retain the tile MI bounds needed by AV2 §7.20.4 `BlockEndX` and `get_tx_skip`
  for active Wiener NS LR source blocks.
- Resolve the §7.20.4 skip-filter classified-luma feature-window source-read
  coordinates through the existing §7.20.2 source selector and record the
  corresponding `LrTxSkip` lookup coordinates.
- Keep source sample values, `LrTxSkip` values, `FilterClass` derivation,
  §7.20.3/§7.20.4 filtering, 10-bit output/storage, reference refresh, and
  successful local decoder mission decode unsupported.

## Capabilities

### New Capabilities

- `lr-classified-wiener-frontier`: track the local decoder mission classified-Wiener
  dependency frontier and its fail-closed diagnostic.

### Modified Capabilities

- `decoder-support`: move the live local decoder mission runtime diagnostic from the generic
  classified-Wiener gate to the new classified dependency/value-read frontier.
- `tile-partition-traversal-boundary`: retain tile MI bounds on active LR source
  blocks so runtime §7.20.4 dependency derivation is spec-bounded.

## Impact

Affected areas: `crates/splot-decode` tile traversal/runtime frontier code,
focused runtime/traversal/CLI tests, `docs/IMPLEMENTATION-MATRIX.toml`, and
OpenSpec artifacts. No dependency graph, public API, encoder, licensing, oracle
fixture, or successful-output claim changes.
