## Context

The general intra decode reconstructs DC and the § 7.13.2.13 smooth luma modes
bit-exactly. Non-DC prediction differs only in the prediction step. Directional
modes add two pieces over the smooth path: the § 5.20.5.3 `y_mode_offset` escape
(the mode is not in the non-directional `y_mode_index` prefix) and the § 7.13.2.8
single directional prediction process.

A single block has no above/left neighbours, so its § 7.13.2.1 edges are pure
fallbacks and (with `enable_intra_edge_filter == 0`, `MrlIndex == 0`) the
§ 7.13.2.x edge-filter / corner-filter / upsample step is a no-op. This makes the
top-left block the smallest correct directional target.

## Decisions

- **Reconstruct the `y_mode_offset` escape faithfully, but only for the top-left
  no-directional-neighbour case.** `get_intra_y_mode_set` has in-frame
  directional-neighbour reorder branches that depend on per-block
  `IntraJointModes` state not yet modelled. At the tile origin both
  `get_joint_mode` neighbours are out of frame (`DC_PRED`), so no directional
  mode is pre-selected and `modeDelta` reduces to the
  `(modeIdx - NON_DIRECTIONAL_MODES_COUNT)`-th `Default_Mode_List_Y` entry,
  biased by `NON_DIRECTIONAL_MODES_COUNT`. The reorder
  (`Reordered_Y_Mode`, `TOTAL_ANGLE_DELTA_COUNT`, `MAX_ANGLE_DELTA`) is verbatim.
- **Reuse the `splot-recon` middle-angle predictor.** pAngle 135 is a § 7.13.2.8
  "middle" angle (`90 < pAngle < 180`). Its derivatives are
  `dx = dy = Dr_Intra_Derivative[45] = 64`, so every projection lands on an
  integer sample (`shift == 0`); the luma IDIF 4-tap `Dr_Interp_Filter[0] =
  {0,128,0,0}` therefore reduces to a sample copy, bit-identical to the
  `enableIdif == 0` bilinear `predict_intra_middle_directional_angle_rect_into`
  for this angle. The decoder only constructs the § 7.13.2.1 edges and calls it.
- **Construct fallback edges explicitly with the shared corner.** The middle
  predictor takes logical `AboveRow[-1..w)` / `LeftCol[-1..h)` edges whose index 0
  is the `-1` sample. For the no-neighbour block: index 0 is the shared corner
  `1 << (BitDepth - 1)` (128 at 8-bit), the remaining above samples are
  `(1 << (BitDepth - 1)) - 1` (127) and left samples are
  `(1 << (BitDepth - 1)) + 1` (129).
- **Gate to the verified subset.** Only `D135_PRED` (the angle with a single-block
  oracle fixture), only with `AngleDeltaY == 0`, only at the top-left 64x64
  superblock (`n4w == 16`, TX_64X64 -> § 5.20.8.2 `get_tx_set` returns
  TX_SET_DCTONLY -> forced DCT_DCT), only with DC chroma. Non-zero angle deltas,
  other directional modes, sub-64x64 directional blocks, neighbour-having blocks
  (where `shift != 0` makes luma IDIF differ from bilinear and the § 7.13.2.8 edge
  synthesis is not a no-op), and directional chroma are rejected before any
  reconstruction so a wrong prediction can never silently produce a
  wrong-but-plausible frame.

## Risks / Trade-offs

- The IDIF-vs-bilinear coincidence is specific to pAngle 135 (`shift == 0` for
  every projection). Other angles, where IDIF genuinely differs from bilinear,
  need their own IDIF-aware predictor and are deferred; the strict pAngle-135 +
  no-neighbour gate keeps the verified subset honest. The fallback-edge
  construction and the escape reconstruction are asserted by the end-to-end oracle
  test (prediction + residual == avmdec == dav2d), the § 8.2.4 `exit_symbol()`
  guard, and the pinned hash; an incorrect edge constant or mode reconstruction
  would fail bit-exactness.
