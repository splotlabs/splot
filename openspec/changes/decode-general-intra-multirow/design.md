## Context

The non-64x64 brick (`DECODE-GENERAL-INTRA-NON64-MULTISB`) generalized the decode
to a single superblock row and conservatively gated height to exactly 64. The
superblock raster loop (§5.20.2.1) it added already iterates both rows and
columns, and the DC reconstruction already reads above neighbours, so the height
gate was the only thing blocking a full superblock grid.

## Decisions

- **Relax the gate, not the loop.** The loop is already 2-D and the DC/SMOOTH
  reconstruction is already neighbour-aware; this change only widens the
  admission predicate from `height == 64` to `height % 64 == 0`.
- **Full-superblock SMOOTH chroma is row-independent.** The prior brick gated
  SMOOTH chroma to full 64x64 superblocks. `clear_block_decoded_flags` (§5.20.2)
  zeroes a full superblock's above-right region and its below-left is decoded
  later, so the §7.13.2.13 sentinels collapse to the edge-clamped last sample at
  any row — no additional sentinel work is needed for multi-row.
- **Verify with a uniform grid fixture.** Distinct multi-row content makes the
  encoder pick a directional luma mode for the diagonal superblock (its value
  matches one neighbour exactly, beating DC's average), which this path rejects
  as `general_intra_unsupported_y_mode`. A uniform 2x2 grid keeps every block
  DC_PRED while still exercising the multi-row raster loop, cross-row above-
  neighbour DC, and full-superblock SMOOTH chroma at row > 0.

## Risks / Trade-offs

- The fixture is uniform, so the luma residual is zero after the first
  superblock; the value is in exercising the multi-row *control flow* (the loop,
  per-row `clear_left_context`, above-neighbour reads), which would fail under
  any superblock-iteration bug. Distinct multi-row content awaits directional
  luma support.
