## Context

The general intra decode reconstructs one 64x64 superblock bit-exactly. The
tile traversal (`decode_general_intra_partition_tree`) pushed a single
partition-tree root at the tile origin. AV2 § 5.20.2.1 `decode_tile()` instead
iterates a raster grid of superblocks across the tile's MI range, each one a
`decode_partition(r, c, SbSize, ...)` root, on one shared symbol decoder and
tile CDF state. Generalizing to that grid is the minimal change that unlocks
frames larger than one superblock.

The frame size, MI dimensions, MI-size partition context, coefficient context,
and reconstruction workspace must all be sized to the real frame rather than the
64x64 constant. `frame_mi_dimensions` and `minimal_partition_frame_facts`
already derive MI dimensions from `tile_info.{mi_row,mi_col}_starts.last()` and
the superblock size, and `TileMiSizeState` / `TileCoeffContextState` are already
parameterized by `(mi_rows, mi_cols)`; only the IVF size gate, the
`is_general_minimal_intra` size check, and `new_general_intra_workspace` were
hardcoded to 64x64.

## Decisions

- **Iterate the § 5.20.2.1 superblock grid.** Replace the single root with the
  nested `for (r = MiRowStart; r < MiRowEnd; r += sbSize4)` /
  `for (c = MiColStart; c < MiColEnd; c += sbSize4)` loop, one partition-tree
  DFS per superblock, `sbSize4 = Num_4x4_Blocks_Wide[SbSize]`. The shared
  `SymbolDecoder`, tile CDFs, `TileMiSizeState`, and frame-spanning
  `TileCoeffContextState` + workspace carry across superblocks so later
  superblocks DC-predict from already-reconstructed neighbours and read the
  evolved neighbour contexts.
- **Clear the left context per superblock row.** § 5.20.2.1 invokes
  `clear_left_context()` at the start of each superblock row; the above context
  persists. A `TileMiSizeState::clear_left_context` resets the left MI-size line
  to the § 6.19.2.1 clear-context sentinel. (For the single-row 128x64 fixture
  this is a no-op after `new`, but it keeps the traversal correct for future
  multi-row frames.)
- **Keep the frozen tier strict.** The IVF size gate is relaxed only to reach
  routing; `validate_frame_core` (frozen `base_q_idx == 255` path) still requires
  exactly 64x64, and `is_general_minimal_intra` accepts only positive multiples
  of 64. Partial right/bottom superblocks (non-multiple sizes needing edge
  clamping) remain rejected.
- **Add SMOOTH chroma, not just DC.** The multi-superblock encoder codes the
  second superblock's residual-free chroma as `SMOOTH_PRED` (resolved from
  `uv_mode` via § 5.20.5.3 `get_intra_uv_mode_set`, which for the
  non-directional luma subset is `Default_Mode_List_Uv[uv_mode]`). Chroma
  reconstruction builds the § 7.13.2.1 `AboveRow` / `LeftCol` edges from the
  partially-built frame's reconstructed neighbours (handling the no-above /
  no-left / no-neighbour fallbacks) and runs the shared `splot-recon`
  § 7.13.2.13 smooth predictor, then adds the (zero) residual. Other non-DC
  chroma modes are rejected before reconstruction.

## Risks / Trade-offs

- The superblock iteration and the chroma SMOOTH edge construction are asserted
  by the end-to-end oracle test (the full frame == avmdec == dav2d) plus the
  § 8.2.4 `exit_symbol()` guard, so a desync or a wrong edge constant fails
  bit-exactness rather than producing a wrong-but-plausible frame. Treating the
  residual-free SMOOTH chroma over uniform neighbours is exact because every
  intra mode collapses to the uniform neighbour value over uniform edges; the
  general SMOOTH edge construction is implemented (not special-cased) so the
  path remains correct for non-uniform neighbours too.
- `clear_left_context()` per superblock row is exercised only by the single-row
  fixture today (no-op there); multi-row frames are a future increment whose
  fixture will assert it directly.
