## Context

The current tile payload path can derive a single work unit, consume the root
partition decision, and stop at a `DecodeBlockFrontier`. The partition CDF
selectors read `MiSizes`, `LeftMiSizes`, and `AboveMiSizes`, but the minimal
runtime currently seeds those arrays as synthetic read-only vectors and no
crate-private model applies the AV2 § 5.20.4.1 updates performed by
`decode_block()`.

This blocks future recursive `read_partition()` progress: any second partition
decision would need neighbor context from decoded blocks, but today that state
cannot be mutated safely or bounded independently.

## Goals / Non-Goals

**Goals:**

- Add a crate-private tile MI-size state object in `splot-decode`.
- Initialize luma/chroma `MiSizes`, `LeftMiSizes`, and `AboveMiSizes` with the
  § 6.19.2.1 clear-context sentinel over superblock-padded context extents.
- Apply checked AV2 § 5.20.4.1 luma MI-size writes for a supplied block region.
- Apply checked AV2 § 5.20.4.1 chroma MI-size writes when the caller supplies a
  chroma block region.
- Return read-only views compatible with the existing partition-context
  traversal API.
- Integrate the state into the minimal runtime frontier without changing
  minimal hash/raw/Y4M output identity.

**Non-Goals:**

- Full `decode_block()` syntax or mode-info modeling.
- Deriving `ChromaMiRow`, `ChromaMiCol`, or `ChromaMiSize` for every partition
  branch.
- Recursive `read_partition()` after the first block frontier.
- Multi-tile or multi-tile-group scheduling.
- Reconstruction expansion, transform/residual parsing, reference refresh,
  public APIs, or external reference decoder integration.

## Decisions

1. **Use a separate `mi_size_state` module.**

   `partition_traversal.rs` is already near the source-line soft limit and is a
   traversal planner, not an owned state container. A new module keeps mutation
   code small, focused, and independently testable while preserving crate
   privacy.

   Alternative considered: add mutable fields to `TilePartitionContextState`.
   That would overload a read-only view type and make future borrow boundaries
   harder when traversal needs both mutation and immutable selector reads.

2. **Store block sizes as validated table indices.**

   The existing `BlockSize` newtype already validates generated AV2 § 9.2
   table indices and exposes `Num_4x4_Blocks_Wide/High`. The state boundary
   should reuse that type instead of accepting unchecked integers internally.

   Alternative considered: store `usize` directly. That would duplicate
   validation and make out-of-range state easier to construct in tests.

3. **Allocate superblock-padded state extents.**

   AV2 § 5.20.3.1 skips only when a partition start is outside `MiRows` /
   `MiCols`. A block may start inside the visible frame and still write its full
   § 5.20.4.1 footprint into the superblock-padded context area initialized by
   § 6.19.2.1. The state boundary should therefore validate visible start
   coordinates separately from padded footprint bounds.

   Alternative considered: exact `MiRows` by `MiCols` allocation. That was
   sufficient for the minimal 64x64 fixture but would reject valid right/bottom
   edge blocks.

4. **Charge padded state before allocation.**

   The minimal runtime derives visible `MiRows` / `MiCols`, but the owned state
   allocates superblock-padded plane grids and neighbor lines. The runtime should
   compute the same padded allocation shape first, charge the padded grid cells
   to the frame-complexity limit, charge the total `usize` entry storage bytes
   to the decoded-frame byte budget, and only then allocate the state.

   Alternative considered: keep checking only visible `MiRows * MiCols`. That
   under-bounds edge-padded frames and misses the actual allocation size.

5. **Accept explicit chroma update facts.**

   AV2 § 5.20.4.1 writes chroma MI-size state when `HasChroma` is true, but
   correct `ChromaMiRow`, `ChromaMiCol`, and `ChromaMiSize` are partition-path
   outputs. This PR should support the write operation when those facts are
   supplied, while leaving broad derivation to later `decode_partition()` work.

   Alternative considered: luma-only. That is simpler but leaves the state
   object incomplete for the exact AV2 write family and delays a small,
   testable part of the same boundary.

6. **Integrate only as a no-output-change minimal runtime check.**

   The minimal runtime can replace its ad hoc initial context vectors with the
   new state view and apply the root luma block update after the traced block
   symbols succeed. The successful hash/raw/Y4M bytes must remain identical;
   failures keep the existing typed runtime error path. Chroma updates remain
   available to future callers that supply explicit chroma block facts.

   Alternative considered: continue traversal after the first block in the same
   PR. That depends on more `decode_block()` state effects and would overclaim
   tile syntax support.

## Risks / Trade-offs

- **Risk: off-by-one or unchecked region writes** -> Mitigation: use checked
  addition, validate every footprint before mutation, and add positive,
  boundary, and overflow tests.
- **Risk: padded context allocation exceeds visible-dimension limits** ->
  Mitigation: compute padded allocation accounting before construction and add
  runtime limit tests for padded grid cells and total MI-state bytes.
- **Risk: state support is mistaken for broad decode-block support** ->
  Mitigation: matrix/docs explicitly keep `tile-payload-decode`,
  `intra-reconstruction`, and broad `decode_tile()` partial.
- **Risk: chroma facts are under-derived** -> Mitigation: make chroma updates
  caller-supplied and optional, with tests for explicit chroma regions and no
  claim that every partition path derives them yet.
- **Risk: minimal output changes accidentally** -> Mitigation: run runtime
  hash/raw/Y4M focused tests and keep reconstruction output untouched.
