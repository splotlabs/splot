## Context

The general intra decode reconstructs a single-block § 7.13.2.8 `D135_PRED`
luma block (`DECODE-GENERAL-INTRA-ANGLE`) but its chroma admission accepts only
`DC_PRED` and `SMOOTH_PRED`. When the minimal-tool avmenc codes a D135 luma block,
its § 5.20.5.3 `read_intra_uv_mode` chooses `uv_mode == 0`, the directional-follow
branch: `get_intra_uv_mode_set(0)` returns `YMode` and the spec sets
`AngleDeltaUV = AngleDeltaY`. For the supported luma D135 (`AngleDeltaY == 0`) the
chroma is `UVMode == D135_PRED`, `AngleDeltaUV == 0` — a plain § 7.13.2.8
middle-angle chroma intra prediction, NOT CfL (`UV_CFL_PRED`) / CCTX / MHCCP.
splot rejected this with `general_intra_non_dc_chroma_mode`. The reconstruction is
the exact chroma companion of the luma D135 path: the same § 7.13.2.8 middle-angle
predictor over the § 7.13.2.1 no-neighbour fallback edges, on the chroma plane.

Confirmed empirically with temporary mode instrumentation (since removed): the
committed fixture decodes `y_mode == 4` (D135), `uv_mode == 0`, resolved
`UVMode == 4` (D135). The mode is a § 7.13 intra prediction, NOT CfL/CCTX/MHCCP,
so it is in scope.

## Decisions

- **Resolve `D135_PRED` to `SupportedChromaMode::D135Follow` only on the
  directional-follow branch.** `supported_chroma_mode` maps the resolved
  `UVMode == D135_PRED` (value 4) to the new `D135Follow` variant ONLY when
  `uv_mode == 0` and the luma is directional — the § 5.20.5.3 branch that returns
  `YMode` with `AngleDeltaUV = AngleDeltaY`. Since only the luma D135 with
  `AngleDeltaY == 0` is supported, the chroma is pAngle 135 with no angle delta.
  The `D135_PRED` value can also appear from the `Default_Mode_List_Uv` scan paired
  with a non-directional luma (`Default_Mode_List_Uv[8] == D135`); that non-follow
  pairing is left deferred (no oracle fixture reaches it).
- **Reuse the luma directional predictor on the chroma plane.** pAngle 135 is a
  § 7.13.2.8 "middle" angle with `dx = dy = Dr_Intra_Derivative[45] = 64`, so every
  projection lands on an integer sample (`shift == 0`) and the chroma IDIF reduces
  to a sample copy — bit-identical to the `enableIdif == 0` bilinear
  `predict_intra_middle_directional_angle_rect_into`. The new
  `reconstruct_general_intra_chroma_directional_first_into` calls the same
  `predict_directional_noneighbour` helper the luma D135 path uses (building the
  § 7.13.2.1 fallback edges with the shared corner) and adds the § 5.20.7.27 chroma
  residual through `reconstruct_general_intra_block_with_prediction` with the chroma
  plane id and `use_tcq == false` (chroma never uses the § 7.14.4 TCQ dqDenom term).
- **Gate to the top-left no-neighbour 64x64 superblock.** Over a real reconstructed
  neighbour edge the `enableIdif == 0` bilinear reduction no longer equals the spec
  IDIF 4-tap interpolation (bilinear equals IDIF only for a flat edge), so the
  directional chroma over a neighbour edge needs the genuine § 7.13.2.8 chroma IDIF
  (a separate brick). The runtime rejects a neighbour-having `D135Follow` chroma
  block with a structured `decode/unsupported-feature` diagnostic
  (`general_intra_directional_chroma_neighbour`).
- **Keep CfL/CCTX/MHCCP out of scope.** Those return `UV_CFL_PRED` at § 5.20.5.3 /
  read separate cross-component syntax (CfL reads luma samples) and are not a plain
  § 7.13 intra prediction; they remain rejected, deferred to separate bricks.

## Risks / Trade-offs

- Over the flat fallback edges the D135 chroma prediction is NOT uniform — it is a
  135-degree anti-diagonal split (upper-right triangle 127, lower-left 129,
  anti-diagonal 128). With the diagonal chroma residual both U and V reconstruct as
  a genuine directional pattern (17 distinct values each), so this exercises the
  real directional chroma predictor, not a flat fallback shortcut. The IDIF-vs-
  bilinear coincidence is specific to pAngle 135 (`shift == 0`); other chroma angles
  and non-zero angle deltas need their own IDIF-aware predictor and are deferred.
  The reconstruction is asserted by the end-to-end oracle test (prediction +
  residual == avmdec == dav2d), the § 8.2.4 `exit_symbol()` guard, and the pinned
  hash; an incorrect edge constant or mode resolution would fail bit-exactness.
