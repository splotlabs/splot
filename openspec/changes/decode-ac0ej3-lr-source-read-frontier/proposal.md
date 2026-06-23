## Why

The ac0ej3 runtime now derives active Wiener NS LR source-bound facts, but the
live fail-closed diagnostic still stops before the §7.20.2 source-read boundary.
The next small decoder brick is to prove that supported active blocks can reach
their caller-provided block-center source selector while remaining fail-closed
before complete Wiener tap reads, chroma luma-source reads, filtering, or output.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER` for the narrow
  source-read handoff after the existing source-bounds frontier.
- Extend the crate-private LR root frontier/runtime handoff to validate
  block-center source sample selection for active Wiener NS blocks using the
  existing `splot-recon` loop-restoration source selection primitive.
- Move the local ac0ej3 runtime diagnostic from
  `unsupported_wienerns_lr_source_bounds` to a new source-read frontier reason.
- Keep complete Wiener tap reads, chroma luma-source reads, §7.20.3 Wiener NS
  filtering, PC-Wiener classification, 10-bit reconstruction/output, reference
  refresh, and successful ac0ej3 decode unsupported.

## Capabilities

### New Capabilities

### Modified Capabilities

- `tile-partition-traversal-boundary`: retain or expose enough active
  source-bound/source-sample state for the supported root LR frontier to reach
  §7.20.2 block-center source selection transactionally.
- `decoder-support`: track the ac0ej3 LR source-read frontier without claiming
  loop-restoration filtering, reconstruction, output, or successful decode.

## Impact

Affected areas: `crates/splot-decode` tile partition/runtime frontier handoff,
minimal-runtime diagnostics, focused traversal/runtime/CLI tests,
implementation/decoder support matrices, and generated status docs. No public
API, dependency graph, licensing, encoder, oracle fixture, or successful output
claim changes.
