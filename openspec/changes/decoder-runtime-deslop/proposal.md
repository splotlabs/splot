## Why

The decoder cleanup should remove runtime sprawl, not add process sprawl. This
change tracks the generic-runtime cleanup as one mission.

## What Changes

- Extract shared runtime helpers for block context, capability diagnostics, intra
  prediction dispatch, residual planning, and inter block prediction.
- Delete private Rustdoc and long comments from touched decoder runtime files.
- Ratchet the implementation-comment budget to the measured post-cleanup count.
- Keep duplication and source-line hard-allowance gates at their measured state.

## Capabilities

### New Capabilities

- `decoder-runtime-deslop`: Generic decoder runtime cleanup and budget ratchet.

### Modified Capabilities

- None.

## Impact

- Affects `splot-decode` minimal runtime internals, `xtask` budget gates, and
  generated tracking docs.
- No public API, AV2 syntax, decoded output, dependency graph, or external
  decoder behavior changes.
