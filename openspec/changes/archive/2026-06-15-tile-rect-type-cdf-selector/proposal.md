## Why

The tile CDF boundary now covers `DoSplitCdf`, `DoSquareSplitCdf`,
`DoExtPartitionCdf`, and `DoUneven4wayPartitionCdf`, but AV2 § 8.3.2 also maps
the `rect_type` syntax element to `TileRectTypeCdf[PlaneStart][ctx]` between
the square-split and extension partition decisions. Adding the generated
`TileRectTypeCdf` rows to the crate-private boundary closes that remaining
partition decision CDF row family without implementing real partition traversal
or context derivation.

Feature ID: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.

## What Changes

- Extend the existing crate-private `splot-decode` tile CDF boundary to copy,
  select, mutate, and average the generated `RectTypeCdf` rows alongside the
  already-supported partition CDF subset.
- Keep all CDF values sourced from generated `splot-core` § 9.3 default tables;
  no values are hand-transcribed from the spec mirror, AVM, or dav2d.
- Add typed selector and typed bounds errors for `TileRectTypeCdf`.
- Expand focused `splot-decode` tests for default copying, selector bounds,
  `SymbolDecoder::read_symbol(cdf)` handoff, and saved CDF copy/average proof.
- Update decoder support/implementation/conformance docs and archive this
  OpenSpec change.

Non-goals:

- No derivation of `rect_type` `ctx` from `bSize`, `LeftMiSizes`,
  `AboveMiSizes`, `Partition_Size_Adjust_Rect_Type`, `r`, or `c`.
- No real `read_partition()`, recursive `decode_partition()`, `decode_tile()`,
  block syntax traversal, tool-enable validation, or `exit_symbol()` after real
  syntax.
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
  `TileRectTypeCdf` as a source-backed crate-private selector.

## Impact

- Code: `crates/splot-decode/src/tile_payload/cdf.rs` only, reusing
  `splot_core::symbol::SymbolDecoder` and generated default CDF tables.
- Docs: decoder roadmap, decoder support matrix/status, implementation matrix,
  feature status, decoder conformance coverage, and OpenSpec decoder-support
  delta.
- APIs: no public API change and no dependency graph change.
- Diagnostics: existing `decode/unsupported-feature` remains the runtime stop
  for unimplemented tile syntax outside this boundary.
