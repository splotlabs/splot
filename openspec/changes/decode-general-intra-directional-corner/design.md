## Context

The § 7.13.2.8 middle-angle directional predictor (D135/D157) reads the
§ 7.13.2.1 corner `AboveRow[-1] == LeftCol[-1]` on its main diagonal. Prior bricks
(`DECODE-GENERAL-INTRA-NEIGHBOUR-DIRECTIONAL` for D135, `DECODE-GENERAL-INTRA-IDIF-D157`
for D157) only worked in the first superblock row, where `haveAbove == 0` and the
corner is the (correct) repeated-first-left sample of the `haveLeft && !haveAbove`
fallback. The `build_directional_middle_edges` `(true, true)` and `(false, true)`
arms (a real above neighbour, `haveAbove == 1`) returned
`UnsupportedDirectionalAboveEdge` because the real corner
`CurrFrame[plane][y-1][x-1]` is not returned by `intra_dc_edges_for_rect`.

## Goals / Non-Goals

- Goal: build the real § 7.13.2.1 corner for the `haveAbove == 1` arms and admit
  the verified row>0 D135 luma + follow chroma position.
- Non-Goal: row>0 D157 (deferred — no fixture), row>0 first-column D135
  (`!haveLeft && haveAbove`, deferred), sub-superblock directional, non-zero angle
  deltas, the directional-neighbour escape reorder, inter, in-loop filters.

## Decisions

- The corner is read in the caller
  (`reconstruct_general_intra_directional_neighbour_block_into`) via the
  current-frame workspace `reconstructed_sample(plane, x-1, y-1)` accessor (added by
  `DECODE-GENERAL-INTRA-GRID`) and passed into the plane-general edge builder. The
  builder only needs the extra corner sample for the `haveLeft && haveAbove` arm;
  the `!haveLeft && haveAbove` arm derives the corner from the real above row
  (`AboveRow[-1] = CurrFrame[plane][y-1][x] = AboveRow[0]`).
- The IDIF luma 4-tap / bilinear chroma kernel (`DECODE-GENERAL-INTRA-IDIF-D157`)
  is reused unchanged; only the edge construction differs by arm. D135's
  `shift == 0` keeps the IDIF a sample copy, so the corner read is the new piece.
- Admission is gated to the fixtured `frontier.r != 0 && frontier.c != 0`
  (`haveLeft && haveAbove`) full-superblock D135 position, mirrored in the chroma
  D135-follow gate, to preserve the verified-subset discipline.

## Risks / Trade-offs

- The fixture's corner value (luma 100) coincides with the flat above row; the test
  asserts the main diagonal equals the real reconstructed corner (100, not the 128
  fallback) and the left-branch propagates the real gradient, so the corner read is
  still proven. A distinct corner value forces the no-neighbour top-left block off
  DC into a directional mode the minimal subset does not yet decode, so the uniform
  top is the cleanest oracle-agreeing construction.
