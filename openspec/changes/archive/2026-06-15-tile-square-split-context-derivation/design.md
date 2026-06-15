## Context

The tile CDF boundary now owns the partition-entry CDF subset and derives the
left/above-neighbor § 8.3.2 contexts for `do_split`, `rect_type`,
`do_ext_partition`, and `do_uneven_4way_partition`. `do_square_split` remains
the last partition-entry selector in that boundary that accepts only a raw
caller-provided context. Unlike the other four contexts, `do_square_split`
depends on the full `MiSizes[PlaneStart][row][col]` grid and `AvailU` /
`AvailL`, not just the `LeftMiSizes` / `AboveMiSizes` edge arrays.

## Goals / Non-Goals

**Goals:**

- Add a crate-private helper that derives `TileDoSquareSplitCdf[0][ctx]` from
  AV2 § 8.3.2 inputs.
- Bounds-check `bSize`, `PlaneStart`, `r - 1`, `c - 1`, `MiSizes` row/column
  indexes, grid block-size values, and final selector context before row access.
- Preserve the existing Tile CDF selector boundary and return
  `TileCdfSelector::DoSquareSplit`.
- Keep the helper usable from future tile-payload / `read_partition()` code.

**Non-Goals:**

- No syntax reads, partition decisions, allowed/implied partition logic, or
  recursive `read_partition()` / `decode_tile()` traversal.
- No mutation or lifecycle management for `MiSizes`, `LeftMiSizes`, or
  `AboveMiSizes`.
- No full § 8.3 syntax-element CDF selection, full Tile/Saved CDF banks,
  `exit_symbol()` after real syntax, CDF copyback/averaging mutation,
  reconstruction, hashes, Y4M output, reference refresh, AVM/dav2d invocation,
  new dependencies, or public API support.

## Decisions

- Add a separate `SquareSplitContextInput<'a>` instead of extending
  `PartitionContextInput<'a>`. The existing input is edge-array based; the
  square-split formula needs a 2D `MiSizes` grid and availability flags.
- Represent `MiSizes` as `[&'a [&'a [usize]]; 2]`: borrowed rows of borrowed
  columns for each `PlaneStart`. This keeps the helper allocation-free and
  lets tests provide tiny fixed grids.
- Enforce the AV2 § 8.3.2 note through the existing
  `checked_square_split_plane()` path: `PlaneStart` must be 0 for
  `TileDoSquareSplitCdf`.
- Add narrow grid/underflow error variants to `TileCdfError` instead of
  panicking or using sentinel indexes.

## Risks / Trade-offs

- `MiSizes` lifecycle remains future work. Mitigation: the helper borrows a
  caller-owned grid and docs continue to mark runtime partition traversal
  partial.
- The `BLOCK_256X256` discriminant is currently not modeled as a shared
  `splot-core` block-size enum. Mitigation: keep a local spec-cited constant
  in the crate-private context module and test the `ctx += 4` branch.
- Some underflow/overflow paths are defensive and hard to hit in normal decoded
  coordinates. Mitigation: focused unit tests exercise `AvailU` with `r == 0`
  and `AvailL` with `c == 0`.
