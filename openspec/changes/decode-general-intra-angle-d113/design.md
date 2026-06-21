## Context

The § 7.13.2.8 MIDDLE-angle zone (`90 < pAngle < 180`) has three angles: D135
(pAngle 135), D157 (pAngle 157), and D113 (pAngle 113). D135 and D157 are decoded
(`DECODE-GENERAL-INTRA-IDIF-D157`, `DECODE-GENERAL-INTRA-DIRECTIONAL-CORNER`);
D113 was rejected. D113 is VERTICAL-LEANING: `dx = Dr_Intra_Derivative[180 - 113]
= Dr_Intra_Derivative[67] = 24`, `dy = Dr_Intra_Derivative[113 - 90] =
Dr_Intra_Derivative[23] = 170` (§ 9.2, matching the committed `Mode_To_Angle[5]
== 113` and the recon kernel's existing `IntraMiddleDirectionalAngle::D113`). The
small `dx` makes most projections take the § 7.13.2.8 ABOVE branch (`base >= -(1 +
mrlIndex)`) with a NONZERO `shift`, so the luma IDIF 4-tap genuinely interpolates
over the real above row + corner — distinct from D135 (`shift == 0`, a copy) and
complementary to D157 (mostly left-branch).

## Goals / Non-Goals

- Goal: admit the verified row>0, non-first-column D113 luma + D113-follow chroma
  position, reusing the existing IDIF kernel and the real § 7.13.2.1 corner.
- Non-Goal: the top-left / first-row (`haveAbove == 0`, degenerate above-fallback)
  / first-column / sub-partitioned / non-64x64 D113 positions; the one-sided
  angles D45/D67/D203 (no IDIF wiring); non-zero angle deltas; the
  directional-neighbour escape reorder; inter; in-loop filters.

## Decisions

- Both prerequisites (the § 7.13.2.8 luma IDIF 4-tap kernel and the real
  § 7.13.2.1 corner builder) already exist and are reused UNCHANGED; this brick
  only adds the mode classification (mode 5 -> D113, pAngle 113), the D113Follow
  chroma resolution, and the admission arms.
- D113 reaches the decode via the § 5.20.5.3 `y_mode_offset` escape
  (`y_mode_offset == 2` -> modeIdx 9 -> modeDelta 29 ->
  `Reordered_Y_Mode[8] == D113_PRED`, `AngleDeltaY == 0`) at § 8.3.2 ctx == 0;
  this is already handled by `reconstruct_y_mode_offset_escape_top_left` (it
  resolves any modeIdx through the shared `resolve_y_mode_top_left`).
- Admission is gated to the fixtured `frontier.r != 0 && frontier.c != 0`
  (`haveLeft && haveAbove`) full-superblock D113 position, mirrored in the chroma
  D113-follow gate, to preserve the verified-subset discipline.

## Risks / Trade-offs

- D113 is vertical-leaning, so a degenerate (flat) above row would make it nearly
  flat. The fixture uses a flat above row (the top-right superblock's uniform 100
  bottom row) + corner 100 but a VARYING real left column (the bottom-left
  superblock's vertical gradient), so the left branch projects the gradient
  up-right and the block is genuinely directional (the bottom row varies across
  columns, not flat / row-constant). The 2940/4096 nonzero-shift count confirms
  the IDIF 4-tap is genuinely exercised over the real above row + corner.
