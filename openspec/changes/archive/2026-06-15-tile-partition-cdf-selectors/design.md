## Context

The existing `DECODE-TILE-CDF-SELECTION-BOUNDARY` row is a crate-private
`splot-decode` boundary. It copies a small mutable tile CDF subset from
generated § 9.3 defaults, validates typed row selectors, hands selected rows to
`SymbolDecoder::read_symbol(cdf)`, and records the § 8.2 copy/average policy.
That boundary currently includes only `DoSplitCdf` and `DoSquareSplitCdf`.

AV2 § 5.20.3.2 `read_partition()` can also read `do_ext_partition` and
`do_uneven_4way_partition`. AV2 § 8.3.2 maps those syntax elements to
`TileDoExtPartitionCdf[PlaneStart][ctx]` and
`TileDoUneven4wayPartitionCdf[PlaneStart][ctx]`, and generated `splot-core`
tables already provide the corresponding § 9.3 defaults.

Spec anchors:

- § 5.20.1 tile payload calls `init_symbol(tileSize)`, `decode_tile()`, and
  later `exit_symbol()`:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`.
- § 5.20.2.1 keeps `decode_tile()` as the unsupported runtime boundary:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1`.
- § 5.20.3.2 `read_partition()` reaches `do_ext_partition` and
  `do_uneven_4way_partition`:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2`.
- § 8.2.2 names Tile CDF copies, including `TileDoExtPartitionCdf` and
  `TileDoUneven4wayPartitionCdf`:
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2-2`.
- § 8.2.4 and § 8.2.6 define copy/average policy and `read_symbol(cdf)`
  mutation behavior:
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2-4` and
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2-6`.
- § 8.3.1/§ 8.3.2 define `S()` CDF row selection and the two new partition
  CDF mappings:
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-1` and
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`.
- § 9.3 default tables provide `Default_Do_Ext_Partition_Cdf` and
  `Default_Do_Uneven_4way_Partition_Cdf`:
  `docs/spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3`.

## Goals / Non-Goals

**Goals:**

- Reuse the existing Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` and
  decoder support row `tile-cdf-selection-boundary`.
- Copy `DEFAULT_DO_EXT_PARTITION_CDF` and
  `DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF` from generated `splot-core` tables
  into the owned frame/tile subset.
- Add typed selectors and typed errors for the two new row families using two
  `PlaneStart` contexts and 64 partition contexts.
- Include the new rows in saved subset copy and average tests so the supported
  subset remains internally complete.
- Keep `splot-decode` crate-private ownership and preserve existing runtime
  unsupported behavior outside the boundary.

**Non-Goals:**

- No `TileRectTypeCdf` selector support. § 8.3.2 lists `rect_type` between the
  existing and newly added rows; this change does not claim complete
  `read_partition()` CDF coverage.
- No derivation of partition contexts from `LeftMiSizes`, `AboveMiSizes`,
  `Partition_Size_Adjust`, `rectType`, or tool-enable syntax.
- No full Tile/Saved CDF bank, real tile completion, `exit_symbol()` after real
  syntax, or `frame_end_update_cdf()`.
- No public API, CLI behavior, dependency, scheduler, reconstruction, hash, Y4M,
  reference refresh, AVM, or dav2d change.

## Decisions

1. **Update the existing boundary row instead of adding a second Feature ID.**

   The implementation surface is the same crate-private CDF boundary and the
   current Feature ID already represents staged expansion of that boundary.
   Updating the row keeps the support matrix honest without pretending runtime
   partition decoding exists.

2. **Reuse the existing dimensions for the new row families.**

   The generated defaults are `[[[i32; 3]; 64]; 2]`, matching the existing
   `DoSplitCdf` partition structure shape. Selector validation should therefore
   reuse the two-plane, 64-context policy, but report each concrete CDF array in
   typed errors.

3. **Keep tests source-backed and table-oriented.**

   Tests compare against generated statics and mutate rows through selectors.
   They must not embed copied CDF table values or rely on AVM/dav2d source.

4. **Preserve the unsupported runtime stop.**

   The boundary can select rows for future syntax reads, but real
   `decode_tile()` still returns structured unsupported metadata until a future
   OpenSpec change implements syntax traversal and reconstruction.

## Risks / Trade-offs

- **Risk: overclaiming full § 8.3 support.** Mitigation: docs, spec delta, and
  matrix notes explicitly keep `TileRectTypeCdf`, context derivation, and
  recursive partition traversal out of scope.
- **Risk: selector panics on untrusted contexts.** Mitigation: all selector
  paths validate `plane_start` and `ctx` before indexing and return typed errors.
- **Risk: table drift or contamination.** Mitigation: copy only from generated
  `splot-core` defaults and compare to those statics in tests.
- **Risk: saved CDF proof falls behind the expanded subset.** Mitigation:
  include the new row families in copy and average coverage.
