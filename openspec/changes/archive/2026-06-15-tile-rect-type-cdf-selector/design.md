## Context

`DECODE-TILE-CDF-SELECTION-BOUNDARY` is a crate-private `splot-decode`
boundary. It owns a small mutable tile CDF subset copied from generated § 9.3
defaults, validates typed row selectors, hands selected rows to
`SymbolDecoder::read_symbol(cdf)`, and records § 8.2 copy/average policy.

After `tile-partition-cdf-selectors`, the boundary covers `DoSplitCdf`,
`DoSquareSplitCdf`, `DoExtPartitionCdf`, and `DoUneven4wayPartitionCdf`.
AV2 § 8.3.2 also maps `rect_type` to
`TileRectTypeCdf[PlaneStart][ctx]`. Generated `splot-core` tables already
provide `DEFAULT_RECT_TYPE_CDF` with the same partition-structure shape used by
the other two-plane partition row families.

Spec anchors:

- § 5.20.1 tile payload calls `init_symbol(tileSize)`, `decode_tile()`, and
  later `exit_symbol()`:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`.
- § 5.20.2.1 keeps `decode_tile()` as the unsupported runtime boundary:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1`.
- § 5.20.3.2 `read_partition()` contains the partition syntax territory that
  remains unsupported:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2`.
- § 8.2.2 names Tile CDF copies, including `TileRectTypeCdf`:
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2-2`.
- § 8.2.4 and § 8.2.6 define copy/average policy and `read_symbol(cdf)`
  mutation behavior:
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2-4` and
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2-6`.
- § 8.3.1/§ 8.3.2 define `S()` CDF row selection and the `rect_type` mapping:
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-1` and
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`.
- § 9.3 default tables provide `Default_Rect_Type_Cdf`:
  `docs/spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3`.

## Goals / Non-Goals

**Goals:**

- Reuse Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` and decoder support row
  `tile-cdf-selection-boundary`.
- Copy `DEFAULT_RECT_TYPE_CDF` from generated `splot-core` tables into the
  owned frame/tile subset.
- Add typed selector and typed errors for `TileRectTypeCdf` using two
  `PlaneStart` contexts and 64 contexts.
- Include `TileRectTypeCdf` in saved subset copy and average tests.
- Preserve crate-private scope and current runtime unsupported behavior outside
  the boundary.

**Non-Goals:**

- No computation of `rect_type` `ctx` from runtime partition state.
- No validation of whether `rect_type` should be read for a given block size,
  shape, or tool configuration.
- No full Tile/Saved CDF bank, real tile completion, `exit_symbol()` after real
  syntax, or `frame_end_update_cdf()`.
- No public API, CLI behavior, dependency, scheduler, reconstruction, hash, Y4M,
  reference refresh, AVM, or dav2d change.

## Decisions

1. **Continue expanding the existing boundary row.**

   `TileRectTypeCdf` is the same crate-private CDF boundary surface as the
   previous partition row families. A second Feature ID would imply a separate
   runtime feature, while this change is still a staged expansion of
   `DECODE-TILE-CDF-SELECTION-BOUNDARY`.

2. **Use the generated table shape directly.**

   `DEFAULT_RECT_TYPE_CDF` is `[[[i32; 3]; 64]; 2]`. Selector validation should
   therefore use the existing two-plane, 64-context path while reporting
   `TileRectTypeCdf` in typed errors.

3. **Keep context derivation out of the selector boundary.**

   § 8.3.2 computes `rect_type` `ctx` from block-size and neighbor partition
   state. This boundary receives a precomputed `ctx`; future `read_partition()`
   work must derive and validate that context before selecting a row.

## Risks / Trade-offs

- **Risk: overclaiming partition decode.** Mitigation: docs and matrix notes
  state that context derivation, `read_partition()`, `decode_tile()`, and full
  § 8.3 remain unsupported.
- **Risk: selector panics on untrusted contexts.** Mitigation: validate
  `plane_start` and `ctx` before indexing and return typed errors.
- **Risk: table drift or contamination.** Mitigation: copy only from generated
  `splot-core` defaults and compare to those statics in tests.
