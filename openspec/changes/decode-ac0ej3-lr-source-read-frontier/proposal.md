## Why

The ac0ej3 runtime now derives active Wiener NS LR source-bound facts, but the
live fail-closed diagnostic still stops before any source frame or §7.20.2
sample selection is attempted. The next small decoder brick is to prove that the
supported active blocks can resolve their caller-provided source samples while
remaining fail-closed before filtering or output.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER` for the narrow
  source-read handoff after the existing source-bounds frontier.
- Extend the crate-private LR root frontier/runtime handoff to resolve source
  sample reads for active Wiener NS blocks using the existing
  `splot-recon` loop-restoration source selection/read primitives.
- Move the local ac0ej3 runtime diagnostic from
  `unsupported_wienerns_lr_source_bounds` to a new source-read frontier reason.
- Keep §7.20.3 Wiener NS filtering, PC-Wiener classification, chroma/luma
  filter application, 10-bit reconstruction/output, reference refresh, and
  successful ac0ej3 decode unsupported.

## Capabilities

### New Capabilities

### Modified Capabilities

- `tile-partition-traversal-boundary`: retain or expose enough active
  source-bound/source-sample state for the supported root LR frontier to attempt
  §7.20.2 source reads transactionally.
- `decoder-support`: track the ac0ej3 LR source-read frontier without claiming
  loop-restoration filtering, reconstruction, output, or successful decode.

## Impact

Affected areas: `crates/splot-decode` tile partition/runtime frontier handoff,
minimal-runtime diagnostics, focused traversal/runtime/CLI tests,
implementation/decoder support matrices, and generated status docs. No public
API, dependency graph, licensing, encoder, oracle fixture, or successful output
claim changes.
