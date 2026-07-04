## Why

The runtime now retains per-unit Wiener NS LR selection state, but the live
`unsupported_active_wienerns_lr_units` diagnostic still points at the superseded
aggregate activity row. That makes the current local decoder mission gate look older than the
state boundary actually reached.

## What Changes

- Keep active Wiener NS LR units fail-closed before decoded-frame allocation,
  reference retention, hash, raw, or Y4M output.
- Reassign the active LR-unit unsupported diagnostic to
  `DECODE-LR-UNIT-SELECTIONS-FRONTIER` /
  `lr-unit-selections-frontier`.
- Update runtime and CLI regression tests plus matrix/support status checks.

## Capabilities

### New Capabilities

### Modified Capabilities

- `decoder-support`: the live local decoder mission active LR-unit diagnostic is owned by the
  selection-state frontier after per-unit state is retained.

## Impact

Affected areas: `crates/splot-decode`, local decoder mission CLI regression, decoder
support / implementation matrices, and status checks. No public API,
dependency graph, decode admission, reconstruction, output, oracle fixture, or
successful decode claim changes.
