## Why

The local decoder mission stream now parses its frame-level Wiener NS filter bank and
then stops before tile syntax because loop restoration still requires
`read_lr()` unit symbols. Modeling the narrow frame-level Wiener NS unit syntax
is the next fail-closed step toward honest reconstruction without claiming
loop-restoration filtering or output support.

## What Changes

- Add Feature ID `DECODE-LR-UNIT-SYNTAX-FRONTIER` to track the narrow
  local decoder mission loop-restoration tile-syntax frontier.
- Extend the crate-private tile partition traversal boundary so supported
  `RESTORE_WIENER_NONSEP` planes can consume AV2 §5.20.10.4/§5.20.10.5
  `read_lr()` / `use_wiener_ns` symbols before partition traversal.
- Keep PC-Wiener, switchable LR unit syntax, per-unit Wiener coefficient reads,
  loop-restoration reconstruction, 10-bit output, and successful local decoder mission decode
  unsupported.
- Update decoder support/status tracking so the emitted runtime diagnostic
  points at a concrete support row after the LR unit syntax is parsed.

## Capabilities

### New Capabilities

### Modified Capabilities
- `tile-partition-traversal-boundary`: narrow support for frame-level Wiener NS
  LR unit syntax before partition traversal.
- `decoder-support`: track the local decoder mission LR unit syntax frontier and its structured
  unsupported diagnostic.

## Impact

Affected areas: `crates/splot-decode` tile CDF rows, tile partition traversal,
minimal runtime diagnostic ordering, focused decoder tests, and the
implementation/decoder support matrices. No public API, dependency graph,
licensing, encoder, or AVM/dav2d invocation changes.
