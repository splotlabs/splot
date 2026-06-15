# Design: Tile Partition Context Derivation

## Current State

`splot-decode` has a crate-private tile CDF boundary that owns a small
partition-entry CDF subset and exposes checked `TileCdfSelector` row access. The
current selectors accept caller-supplied `ctx` values. That proves row storage,
bounds, mutation handoff, and saved CDF copy/average policy, but it does not
derive the § 8.3.2 contexts from tile-neighbor state.

## Approach

Add `crates/splot-decode/src/tile_payload/cdf/context.rs` as a child module of
the existing boundary. The child module will:

- define a crate-private `PartitionContextInput<'a>` with `b_size`,
  `plane_start`, `r`, `c`, `left_mi_sizes`, and `above_mi_sizes`;
- define crate-private `RectPartitionType` for the horizontal/vertical
  direction needed by extended partition contexts;
- use generated `splot-core` § 9.2 tables for `Mi_Width_Log2`,
  `Mi_Height_Log2`, `Num_4x4_Blocks_Wide`, and `Num_4x4_Blocks_High`;
- define only the two small § 8.3.2 local adjustment arrays that are not
  currently generated: `Partition_Size_Adjust` and
  `Partition_Size_Adjust_Rect_Type`;
- validate `b_size`, `plane_start`, `r`, `c`, second-half ext offsets, and
  neighbor block-size entries before every lookup;
- return existing `TileCdfSelector` values for `do_split`, `rect_type`,
  `do_ext_partition`, and `do_uneven_4way_partition`.

## Error Handling

Extend the existing crate-private `TileCdfError` with narrow context-derivation
variants instead of panicking:

- invalid block-size index;
- missing left/above neighbor slot;
- arithmetic overflow while deriving the second ext-partition neighbor index.

The existing `SelectorOutOfRange` error remains the final guard for selector
dimensions.

## Non-Goals And Boundaries

This change is still selector-boundary infrastructure. It does not decode or
parse partition syntax. In particular it does not:

- derive `do_square_split` context, because that requires the full `MiSizes`
  grid plus `AvailU`/`AvailL`;
- decide whether a syntax element is present;
- call `SymbolDecoder::read_symbol`, `S()`, or `L(1)`;
- return partition types;
- recurse through `read_partition()` or `decode_tile()`;
- update `LeftMiSizes` / `AboveMiSizes` after blocks;
- mutate saved CDFs after tile completion;
- reconstruct pixels or produce runtime outputs.

## Risks

The main risk is overclaiming. Documentation and matrix text must say only that
left/above-derived contexts for four existing selector families are modeled.
`do_square_split` context derivation, full § 8.3 selection, and real
`read_partition()` remain future work.
