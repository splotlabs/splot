## Context

> **Historical note.** This change predates `decode-minimal-fixture-avm-skip-polarity`, which retired the frozen `base_q_idx == 255` committed minimal fixture (`syn-flat-intra-64x64-minimal.ivf`) and replaced it with an AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream that routes through the general intra path. References below to the committed minimal fixture as the frozen `base_q_idx == 255` anchor — and to keeping that committed fixture's hash byte-identical — are historical; the routing rule (a `base_q_idx == 255` frame falls through to the frozen gate) still holds.

The multi-row brick (`DECODE-GENERAL-INTRA-MULTIROW`) generalized the decode to a
single superblock row OR column and conservatively rejected 2-D grids. The reason:
the §7.13.2.13 SMOOTH chroma top-right sentinel `AboveRow[w]` was built by
edge-clamping (repeating the last in-block above sample), which equals the spec
value only when the above-right is not decoded. In a 2-D grid a non-rightmost
row>0 superblock's above-right IS decoded, so the sentinel must read the real
reconstructed neighbour.

## Decisions

- **Read the real §7.13.2.1 above-right sentinel.** Per §7.13.2.1, when
  `haveAbove == 1` the top-right sentinel is
  `CurrFrame[plane][y - 1][Min(aboveLimit, x + w)]` with
  `aboveLimit = Min(maxX, x + w + 4 * num4AboveRight - 1)`. For `num4AboveRight >= 1`
  this simplifies to the column `Min(maxX, x + w)` — the real reconstructed sample
  one column past the block's right edge. When `num4AboveRight == 0` (no decoded
  above-right) or the block touches the chroma frame right edge, the sentinel
  collapses to the clamped last in-block sample, which the existing edge-clamp
  already supplies.

- **Derive `num4AboveRight` faithfully to §5.20.7.25 `count_top_right_avail`.**
  For a full 64x64 superblock the block coincides with the superblock, so its
  chroma sub-block MI position within the superblock is `(0, 0)` and its chroma
  width is `w4 = n4w >> SubsamplingX`. `count_top_right_avail(plane, 0, 0, w4)`
  scans `BlockDecoded[plane][-1][w4 + i]` for `i in 0..w4`; per §5.20.2.3
  `clear_block_decoded_flags` the above row is decoded for chroma columns
  `x < (MiColEnd - c) >> SubsamplingX` (a single full-frame tile has
  `MiColEnd == MiCols`), so a column `w4 + i` is decoded while
  `w4 + i < (MiCols - c) >> SubsamplingX`, stopping at the first undecoded column.

- **Keep the bottom-left `LeftCol[h]` sentinel as the clamp.** In raster decode
  order a full-superblock block's below-left is never decoded yet
  (`num4BelowLeft == 0`), so the spec value
  `CurrFrame[plane][Min(maxY, y+h)][x-1]` equals the clamped last left sample. No
  change is needed; the SMOOTH chroma builder documents this with a spec cite.

- **Add a total/checked workspace accessor.** `resolve_smooth_above_right_sentinel`
  reads the sentinel via a new `CurrentFrameWorkspace::reconstructed_sample`
  accessor (splot-recon), which validates the column against the storage width
  (not just the flat index) and returns a `Result` — no panic, no aliasing into
  the next row.

- **Keep SMOOTH chroma gated to full superblocks.** Sub-partitioned SMOOTH chroma
  needs the per-block §5.20.2.3 `BlockDecoded` update (so an intra-superblock
  above-right / below-left split child is read correctly), which is a separate
  brick.

- **Verify with a 2-D fixture the fix actually changes.** `syn-grid-intra-128x128`
  has uniform luma (every superblock DC) and distinct flat chroma per quadrant,
  except a SMOOTH bottom-left superblock whose decoded above-right (the top-right
  quadrant) differs from its own top edge. With the old repeat-last sentinel
  splot mismatched the oracle by 4096 bytes; with the real above-right read it is
  bit-exact (the bottom-left's top-right corner is pulled toward 200, not 110).

## Risks / Trade-offs

- Sub-partitioned SMOOTH chroma and the per-block `BlockDecoded` update are still
  deferred. The full-superblock `num4AboveRight` derivation is exact for the
  admitted subset; it is not a general per-transform-block `count_top_right_avail`
  (which the later sub-partitioned brick will add).
