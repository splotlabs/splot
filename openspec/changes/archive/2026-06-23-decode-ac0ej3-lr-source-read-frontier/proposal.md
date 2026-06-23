## Why

The ac0ej3 runtime now derives active Wiener NS LR source-bound facts, but the
live fail-closed diagnostic still stops before the §7.20.2 source-read boundary.
The next small decoder brick is to prove that supported active blocks can resolve
their output, Wiener tap, and chroma luma-source coordinates through the
caller-provided source selector while remaining fail-closed before source sample
value reads, filtering, or output.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER` for the narrow
  source-read handoff after the existing source-bounds frontier.
- Extend the crate-private LR root frontier/runtime handoff to validate source
  sample selection for active Wiener NS output, tap, and luma-source coordinates
  using the existing `splot-recon` loop-restoration source selection primitive.
- Move the local ac0ej3 runtime diagnostic from
  `unsupported_wienerns_lr_source_bounds` to a new source-read frontier reason.
- Keep source sample value reads, §7.20.3 Wiener NS filtering, PC-Wiener
  classification, 10-bit reconstruction/output, reference refresh, and
  successful ac0ej3 decode unsupported.

## Capabilities

### New Capabilities

### Modified Capabilities

- `tile-partition-traversal-boundary`: retain or expose enough active
  source-bound/source-sample state for the supported root LR frontier to reach
  §7.20.2 output, tap, and luma-source selection transactionally.
- `decoder-support`: track the ac0ej3 LR source-read frontier without claiming
  loop-restoration filtering, reconstruction, output, or successful decode.

## Impact

Affected areas: `crates/splot-decode` tile partition/runtime frontier handoff,
minimal-runtime diagnostics, focused traversal/runtime/CLI tests,
implementation/decoder support matrices, generated status docs, and the
`DecodeLimits` source-read operation budget. No dependency graph, licensing,
encoder, oracle fixture, or successful output claim changes.
