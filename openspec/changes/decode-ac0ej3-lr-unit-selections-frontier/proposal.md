## Why

The ac0ej3 runtime now consumes supported frame-level Wiener NS LR-unit syntax
and distinguishes inactive units from active `RESTORE_WIENER_NONSEP` units, but
it preserves only aggregate counts. Future loop-restoration reconstruction needs
the per-unit selection state in syntax order, including plane and LR-unit
coordinates, before it can route active units to the §7.20 filtering process.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-LR-UNIT-SELECTIONS-FRONTIER` for the narrow
  LR-unit selection-state frontier.
- Extend the crate-private LR root frontier to retain each supported
  frame-level Wiener NS LR-unit selection as `(plane, unit_row, unit_col,
  active)`.
- Keep the minimal runtime fail-closed for active units with the existing
  `unsupported_active_wienerns_lr_units` diagnostic.
- Keep active loop-restoration filtering, 10-bit reconstruction/output,
  PC-Wiener, switchable LR, temporal/reference Wiener state, and successful
  ac0ej3 decode unsupported.

## Capabilities

### New Capabilities

### Modified Capabilities

- `tile-partition-traversal-boundary`: retain supported frame-level Wiener NS
  LR-unit selections in syntax order while preserving aggregate activity counts
  and transactional CDF behavior.
- `decoder-support`: track the ac0ej3 LR-unit selection-state frontier without
  claiming loop-restoration reconstruction.

## Impact

Affected areas: `crates/splot-decode` tile partition traversal summaries,
focused traversal tests, and implementation/decoder support matrices. No public
API, dependency graph, licensing, encoder, oracle fixture, or successful output
claim changes.
