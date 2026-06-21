## Context

> **Historical note.** This change predates `decode-minimal-fixture-avm-skip-polarity`, which retired the frozen `base_q_idx == 255` committed minimal fixture (`syn-flat-intra-64x64-minimal.ivf`) and replaced it with an AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream that routes through the general intra path. References below to the committed minimal fixture as the frozen `base_q_idx == 255` anchor — and to keeping that committed fixture's hash byte-identical — are historical; the routing rule (a `base_q_idx == 255` frame falls through to the frozen gate) still holds.

The non-64x64 brick (`DECODE-GENERAL-INTRA-NON64-MULTISB`) generalized the decode
to a single superblock row and conservatively gated height to exactly 64. The
§5.20.2.1 superblock raster loop it added already iterates both rows and columns,
and the DC reconstruction already reads above neighbours, so a single superblock
COLUMN works too.

## Decisions

- **Admit a single row OR a single column, not a 2-D grid.** The safe condition
  for the existing edge-clamped SMOOTH chroma sentinel is that no full-superblock
  block has a decoded above-right neighbour. That holds exactly when every
  superblock is row-0 (no above) or rightmost-column (no above-right) — i.e. the
  frame is a single row (height == 64) or a single column (width == 64).
- **Reject 2-D grids.** Per §5.20.2 `clear_block_decoded_flags`, a non-rightmost
  superblock's above row is marked decoded up to `(MiColEnd - c) >> subX`, which
  exceeds the superblock width, so a row>0 non-rightmost superblock's above-right
  IS decoded and `count_top_right_avail` makes §7.13.2.1 use the real
  `CurrFrame[y-1][x+w]` for `AboveRow[w]`. The current SMOOTH chroma path repeats
  the last in-block sample instead, so a 2-D grid with non-uniform chroma would
  mispredict — it is rejected until the sentinel reads the real neighbour.
- **Verify with a distinct-value single-column fixture.** `syn-2sbcol-intra-64x128`
  stacks two distinct flat superblocks (top 80, bottom 180), so the bottom
  superblock DC-predicts from the reconstructed top neighbour with a real residual
  and reconstructs full-superblock SMOOTH chroma at row > 0 (rightmost column, no
  above-right) — exercising the multi-row control flow with a non-trivial value.

## Risks / Trade-offs

- 2-D grid support (the common real case) is still deferred; it requires reading
  the real §7.13.2.1 above-right sentinel from the reconstructed frame, a separate
  brick. This change is a verified-safe stepping stone (single row or column).
