## Why

The ac0ej3 runtime now consumes Wiener NS LR-unit selections, but the live
diagnostic still stops before any §7.20 loop-restoration block geometry is
derived. The next small decoder brick is to complete the required §5.20.10.6
per-unit Wiener NS filter syntax, then prove active units can be mapped to
caller-resolved §7.20.1 source bounds, including the sequence tile-boundary
flag, without reading source frames or filtering output.

## What Changes

- Add Feature ID `DECODE-AC0EJ3-LR-SOURCE-BOUNDS-FRONTIER` for the narrow
  active LR source-bounds frontier.
- Extend the crate-private LR root frontier to retain active Wiener NS
  loop-restore blocks with `unitRow`/`unitCol`, current-plane block coordinates,
  block size, and luma source/stripe bounds from AV2 §7.20.1.
- Add the Wiener NS length, UV-symmetry, and base CDF rows needed to consume the
  §5.20.10.6 entropy-coded per-unit filter syntax reached by ac0ej3 chroma
  planes; do not expose or apply the decoded coefficients.
- Move the local ac0ej3 runtime diagnostic from the active unit-selection gate
  to `unsupported_wienerns_lr_source_bounds`.
- Keep source-frame reads, §7.20.2 sample reads, §7.20.3 filtering, 10-bit
  reconstruction/output, PC-Wiener classification, chroma filtering, and
  successful ac0ej3 decode unsupported.

## Capabilities

### New Capabilities

### Modified Capabilities

- `tile-partition-traversal-boundary`: retain active frame-level Wiener NS
  source-bound facts for the supported root LR frontier after consuming the
  required per-unit Wiener NS filter syntax.
- `decoder-support`: track the ac0ej3 LR source-bounds frontier without
  claiming loop-restoration reconstruction or output support.

## Impact

Affected areas: `crates/splot-decode` tile partition traversal summaries,
minimal-runtime diagnostics, focused traversal/runtime/CLI tests, and
implementation/decoder support matrices. No public API, dependency graph,
licensing, encoder, oracle fixture, or successful output claim changes.
