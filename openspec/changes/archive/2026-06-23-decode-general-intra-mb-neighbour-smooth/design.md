## Context

The general intra decode reconstructs DC and the § 7.13.2.13 smooth / § 7.13.2.8
directional luma modes bit-exactly, but every non-DC / directional luma block is
gated to the no-neighbour top-left block, where the § 7.13.2.1 edges are pure
flat fallbacks. The next correct multi-block step is a non-DC luma block that
reads the genuine reconstructed neighbour edge of an already-decoded block.

The committed `syn-mbvg-128x64-q80.ivf` is two side-by-side 64x64 superblocks of a
vertical luma gradient. Temporary mode instrumentation confirmed both superblocks
decode as § 7.13.2.13 `SMOOTH_V_PRED` (canonical mode 10, via the § 5.20.5.3
`y_mode_offset` escape), NOT the cardinal `H_PRED`/`V_PRED` the brief anticipated.

## Decisions

- **Implement the actual mode (SMOOTH_V), not the anticipated cardinal mode.**
  `SMOOTH_V` reads the real neighbour edge but is § 7.13.2.13 linear
  interpolation, not a § 7.13.2.8 angle copy, so no IDIF is involved and the
  result is bit-exact even over a non-flat edge. (A § 7.13.2.8 directional angle
  over a real non-flat neighbour edge would need the real IDIF 4-tap
  interpolation, since the `enableIdif == 0` bilinear reduction equals IDIF only
  for a flat edge — that is a separate brick and stays rejected.) The Feature ID
  `DECODE-GENERAL-INTRA-MB-NEIGHBOUR-SMOOTH` is kept as the agreed brick id; the
  capability content describes SMOOTH_V faithfully.

- **Reuse the existing § 7.13.2.1 edge builder.** The chroma SMOOTH path already
  builds the § 7.13.2.1 `LeftCol[0..=h]` / `AboveRow[0..=w]` edges from the
  partially-built frame (real reconstructed left column / above row, bottom-left
  sentinel `LeftCol[h]` clamped to the last in-block sample since
  `num4BelowLeft == 0` in raster order, top-right sentinel `AboveRow[w]`, and the
  no-above / no-left / no-neighbour fallbacks). That derivation is
  plane-independent, so it is generalized in name only (`build_smooth_chroma_edges`
  -> `build_smooth_edges`, `full_sb_chroma_num4_above_right` ->
  `full_sb_num4_above_right`) and reused for luma with `sub_x == 0`. For the right
  superblock (`haveLeft == 1`, `haveAbove == 0`) § 7.13.2.1 sets
  `LeftCol[i] = CurrFrame[0][Min(leftLimit, y+i)][x-1]` and
  `AboveRow[i] = CurrFrame[0][y][x-1]` — exactly what the builder produces. For
  `SMOOTH_V` the output `predV2` depends only on `AboveRow[j]` and the bottom-left
  sentinel `LeftCol[h]`; the top-right sentinel is irrelevant.

- **The `y_mode_index` § 8.3.2 context stays 0 — no per-MI joint-mode array.**
  § 5.20.5.3 sets `IntraJointMode = modeDelta` (the reorder index, NOT the
  canonical mode value). The left-neighbour SMOOTH_V has `modeDelta == 2`, which
  is `< NON_DIRECTIONAL_MODES_COUNT == 5`, so `get_joint_mode(0) >= 5` is false
  and contributes 0 to the context; the above neighbour is out of frame
  (`DC_PRED`, contributes 0). The reorder branch of `get_intra_y_mode_set`
  (line § 5.20.5.3) only pre-selects a directional mode when a joint-mode
  neighbour is `>= 5`, so the top-left no-directional-neighbour simplification
  (`reconstruct_y_mode_offset_escape_top_left`) is also correct here. A real
  per-MI `IntraJointModes` array is only needed when a *directional* neighbour is
  in frame; that remains deferred (tracked by `DECODE-GENERAL-INTRA-ANGLE`).

- **Gate to a full 64x64 superblock.** A sub-superblock split block needs the
  per-block § 5.20.2.3 `BlockDecoded` update (for the intra-superblock above-right
  / below-left split neighbours), which is not yet modelled, so the
  neighbour-edge non-DC path is gated to `n4w == 16`. Neighbour-having directional
  (D135) luma, sub-superblock non-DC, SMOOTH / PAETH luma, and non-DC chroma
  beyond the existing SMOOTH chroma path are rejected before any reconstruction.

## Risks / Trade-offs

- The bit-exactness of the neighbour-edge read rests on the § 7.13.2.1 edge
  derivation (real left column, the clamped bottom-left sentinel, the repeated
  no-above fallback). It is asserted end-to-end by the oracle test (prediction +
  residual over the real neighbour == avmdec == dav2d, both verified locally), the
  § 8.2.4 `exit_symbol()` guard, and the pinned hash; an incorrect edge derivation
  would fail bit-exactness. The right superblock's reconstruction differs from the
  left's (its centre is 121 vs the left's 122), confirming it reads the real
  neighbour and is not a copy.
