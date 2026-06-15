## Why

The current tile CDF boundary proves the first § 8.3 partition-entry handoff
only for `DoSplitCdf` and `DoSquareSplitCdf`. AV2 § 5.20.3.2 `read_partition()`
also reaches `do_ext_partition` and `do_uneven_4way_partition`, and § 8.3.2
maps those syntax elements to generated tile CDF rows. Extending the
crate-private boundary to those rows is the next source-backed decoder step
before any real partition traversal can consume mutable CDF rows.

Feature ID: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.

## What Changes

- Extend the existing crate-private `splot-decode` tile CDF boundary to copy,
  select, mutate, and average the generated `DoExtPartitionCdf` and
  `DoUneven4wayPartitionCdf` rows alongside the existing `DoSplitCdf` and
  `DoSquareSplitCdf` rows.
- Keep all CDF values sourced from generated `splot-core` § 9.3 default tables;
  no table values are hand-transcribed from the spec mirror, AVM, or dav2d.
- Add typed selector variants and typed bounds errors for the new rows.
- Expand self-contained `splot-decode` tests for default copying, selector
  bounds, `SymbolDecoder::read_symbol(cdf)` handoff, and saved CDF
  copy/average proof for the supported subset.
- Update the decoder support matrix, implementation matrix, generated status
  docs, and decoder roadmap to describe the expanded subset and residuals.

Non-goals:

- No `TileRectTypeCdf` selector support in this change.
- No real `read_partition()`, recursive `decode_partition()`, `decode_tile()`,
  block syntax traversal, context derivation, tool-enable validation, or
  `exit_symbol()` after real syntax.
- No full § 8.3 CDF selection, full Tile/Saved CDF banks, frame-end
  `frame_end_update_cdf()` mutation, reconstruction, hashes, runtime Y4M
  output, reference refresh, public API support, dependency graph change, or
  scheduler change.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or runtime
  invocation.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: Expand the existing tile CDF selection boundary
  requirement for `DECODE-TILE-CDF-SELECTION-BOUNDARY` to include
  `DoExtPartitionCdf` and `DoUneven4wayPartitionCdf` as source-backed
  crate-private selectors.

## Impact

- Code: `crates/splot-decode/src/tile_payload/cdf.rs` only, reusing
  `splot_core::symbol::SymbolDecoder` and generated default CDF tables.
- Docs: decoder roadmap, decoder support matrix/status, implementation matrix,
  feature status, and OpenSpec decoder-support delta.
- APIs: no public API change and no dependency graph change.
- Diagnostics: existing `decode/unsupported-feature` remains the runtime stop
  for unimplemented tile syntax; this change narrows the internal CDF boundary
  only.
