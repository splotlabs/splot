## Context

The single-block general intra decode reads one partition symbol (NONE) then
one block. A partitioned frame interleaves partition-split symbols with per-block
syntax in one bitstream, and each non-first block predicts from and updates the
neighbour state of earlier blocks. The partition traversal primitives, the
MI-size partition context (`TileMiSizeState`), the coefficient neighbour context
(`TileCoeffContextState` with `update_after_coeffs`), and the workspace DC
prediction (`predict_intra_dc_rect_value` over `intra_dc_edges_for_rect`) all
already exist; the gap is a driver that interleaves them correctly.

## Goals / Non-Goals

**Goals:**
- Walk the full §5.20.3.1 partition tree, decoding every leaf block in order.
- Maintain the three neighbour states across blocks: MI-size partition context,
  coefficient context lines, and the reconstructed-sample workspace.
- DC-predict each non-first block from its reconstructed neighbours.
- Decode the four-quadrant fixture bit-exactly to the avmdec/dav2d oracle.

**Non-Goals:**
- No non-DC intra prediction modes, rectangular-leaf partitions, multiple tiles,
  or non-64x64 frames.
- No chroma `cctx`/CfL, inter prediction, or in-loop filters.
- No live in-CI AVM/dav2d dependency.

## Decisions

1. Interleave partition symbols and block decode in one DFS driver.

   Rationale: AV2 §5.20.3.1 reads a partition symbol, then recurses; at a leaf it
   calls decode_block (which reads modes + coefficients). Partition symbols and
   block data are interleaved in the bitstream, so the driver must decode each
   block inline at its leaf before reading the next sibling's partition symbol.
   The driver operates directly on the tile CDFs (no clone) so partition and
   block symbols share the same evolving state.

2. DC-only square gate makes mode contexts trivial.

   Rationale: when every block is DC_PRED, every neighbour's joint mode is
   DC_PRED, so the §8.3.2 y_mode_index and uv_mode contexts both reduce to the
   tile-origin context 0 regardless of position. The existing tile-origin mode
   decode is therefore correct for every block; a non-DC block (which would
   change the context) is rejected before it can desynchronise later blocks.

3. The nonzero coefficient pass already threads dc_sign + the context update.

   Rationale: `apply_coeff_use_fsc_branch_from_frame_facts` reads dc_sign from the
   persistent context at `geometry.start_x >> 2` and commits its own
   `update_after_coeffs`. The driver only derives the `txb_skip` context from the
   neighbour lines and commits the all-zero context write.

## Risks / Trade-offs

- [Risk] Any of the three neighbour states being wrong desynchronises the
  arithmetic decoder.
  -> Mitigation: §8.2.4 `exit_symbol()` after the whole tile is a strong
  bit-exactness check; the single-block q80/cos fixtures are a regression guard
  through the unified driver; the quad fixture is the multi-block oracle anchor.
- [Risk] Verified only for square DC blocks.
  -> Mitigation: non-DC and non-square leaves are rejected with a structured
  diagnostic; the matrix/support rows enumerate the unhandled cases.
