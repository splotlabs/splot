## Why

The local decoder mission stream now reaches the frame-level Wiener NS LR-unit syntax
frontier, but the runtime discards each `use_wiener_ns` value and always reports
loop-restoration reconstruction as unsupported. The next safe step is to record
whether any consumed LR unit actually selects Wiener NS filtering: inactive
units are a no-op, while active units still require reconstruction support.

## What Changes

- Add Feature ID `DECODE-INACTIVE-LR-UNITS-FRONTIER` to track the narrow
  local decoder mission LR-unit activity frontier.
- Extend the crate-private LR-unit traversal summary so callers can distinguish
  consumed `RESTORE_NONE` units from units selecting `RESTORE_WIENER_NONSEP`.
- Let the minimal runtime continue past the LR frontier only when every covered
  frame-level Wiener NS unit is inactive.
- Keep active Wiener NS filtering, PC-Wiener, switchable LR, per-unit Wiener
  coefficient parsing without frame filters, 10-bit reconstruction/output, and
  successful local decoder mission decode unsupported.
- Update decoder support/status tracking and the local decoder mission regression so the
  current diagnostic reflects the next true frontier after inactive LR units.

## Capabilities

### New Capabilities

### Modified Capabilities
- `tile-partition-traversal-boundary`: report frame-level Wiener NS LR-unit
  activity while preserving transactional CDF handling and unsupported active
  filtering boundaries.
- `decoder-support`: track the local decoder mission inactive LR-unit frontier and its current
  structured runtime diagnostic.

## Impact

Affected areas: `crates/splot-decode` tile partition traversal summaries,
minimal runtime frontier ordering, focused decoder/CLI tests, and the
implementation/decoder support matrices. No public API, dependency graph,
licensing, encoder, AVM/dav2d invocation, or successful output claim changes.
