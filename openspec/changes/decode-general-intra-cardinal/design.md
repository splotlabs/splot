## Context

The general intra decode reconstructs the § 7.13.2.8 `D135_PRED` ("middle" angle)
directional luma mode and its `uv_mode == 0` directional-follow D135 chroma, but
NOT the two CARDINAL directional modes `V_PRED` (pAngle 90) and `H_PRED`
(pAngle 180). A V/H luma block was rejected (`general_intra_unsupported_y_mode`,
because the typed `YMode` reconstruction did not map the selecting `y_mode_index`)
and its follow chroma with `general_intra_non_dc_chroma_mode`.

## Decisions

- **Cardinal V/H are a degenerate copy — no IDIF, no corner, no `useIBP`.**
  § 7.13.2.8 step 4 is `pred[i][j] = AboveRow[j]` (V_PRED) and step 5 is
  `pred[i][j] = LeftCol[i]` (H_PRED): a direct sample copy of the § 7.13.2.1 above
  row / left column, reading no other edge. § 7.13.2.7 computes `useIBP` only when
  `pAngle < 90 || pAngle > 180`, so it is 0 for 90/180; its edge-filter ordered
  step is also skipped entirely for `pAngle == 90 || pAngle == 180`; and the MRL
  secondary blend needs `MrlIndex > 0` (0 in the minimal subset). So the copy is
  bit-exact over a NON-flat reconstructed edge with no interpolation — unlike a
  middle angle (D135), where the copy is exact only because `shift == 0`. The
  existing `splot-recon` `predict_intra_cardinal_directional_rect_into`
  (`IntraCardinalDirection::Vertical` / `Horizontal`) already implements both
  steps, so no new prediction kernel is needed; the only new decode work is reading
  the real § 7.13.2.1 edge and resolving the typed mode.

- **V/H at ctx == 0 are coded via the DIRECT first-mode-set `y_mode_index`, NOT the
  `y_mode_offset` escape.** The original brief assumed the escape; it is wrong for
  the cardinal delta-0 case. The § 5.20.5.3 `y_mode_offset` escape (`y_mode_set ==
  0`, `y_mode_index == MODE_INDEX_COUNT - 1`, plus `y_mode_offset` 0..5) reaches
  `modeIdx` 7..12, whose `Default_Mode_List_Y[2..7]` entries are D45/D67/D113/D135/
  D157/D203 — V_PRED (canonical 1) and H_PRED (canonical 2) are at
  `Default_Mode_List_Y[12]`/`[28]`, far outside the escape range. With
  `AngleDeltaY == 0`, the spec instead codes V_PRED at `y_mode_index == 5`
  (`get_intra_y_mode_set(5)` → `Default_Mode_List_Y[0] == 17` → `modeDelta 22` →
  `Reordered_Y_Mode[7] == V_PRED`) and H_PRED at `y_mode_index == 6`
  (`Default_Mode_List_Y[1] == 45` → `modeDelta 50` → `Reordered_Y_Mode[11] ==
  H_PRED`). Both are the DIRECT first-mode-set path (`modeIdx == y_mode_index`, no
  `y_mode_offset` read). The `y_second_mode` (`y_mode_set != 0`) path codes V/H only
  with a NON-zero angle delta (verified by enumeration), so it stays out of scope.
  Empirically: avmenc with `--enable-smooth-intra=0` over a perfect vertical /
  horizontal continuation codes `y_mode_index == 5 / 6` at base_q_idx 160 / 180 (at
  lower QP it preferred an angled `y_second_mode` V/H).

- **Reuse the escape's ctx == 0 selection.** At ctx == 0 the § 5.20.5.3
  `get_intra_y_mode_set` selection loop pre-selects no neighbour mode, so the
  direct first-set directional resolution is identical to the escape's. The
  refactor extracts `resolve_y_mode_top_left(mode_idx)` (shared by the escape and
  the new `reconstruct_y_mode_first_set_directional_top_left(y_mode_index)`), so
  there is one bit-exact resolution. ctx != 0 needs the directional-neighbour
  reorder and is rejected (`general_intra_directional_neighbour_reorder`).

- **Verified subset = neighbour-having full superblock only.** V_PRED needs a real
  above row (`haveAbove == 1`), so it is admitted only on a row>0 full 64x64
  superblock (`frontier.r != 0`, `n4w == 16`); H_PRED needs a real left column
  (`haveLeft == 1`), admitted only on a non-first-column full superblock
  (`frontier.c != 0`, `n4w == 16`). A first-superblock-row V_PRED / first-column
  H_PRED would read the § 7.13.2.1 no-neighbour fallback (no oracle fixture), so it
  is rejected (`general_intra_cardinal_vertical_unverified` /
  `general_intra_cardinal_horizontal_unverified`). The 4:2:0 `uv_mode == 0` follow
  chroma resolves to `UVMode == V_PRED` / `H_PRED` (`AngleDeltaUV == 0`) and reuses
  the same cardinal recon at the half-resolution position (same neighbour
  availability), gated identically.

## Risks / Trade-offs

- **Narrow admission deliberately rejects the encoder's own legal outputs (first-row
  V_PRED, the `y_second_mode` V/H delta, ctx != 0).** That is the verified-subset
  discipline: admit only what a committed oracle fixture proves bit-exact; reject the
  rest with a structured diagnostic so the decoder is never confidently wrong.

## Migration

None. No public API or dependency-graph change; the recon cardinal predictor and the
ctx == 0 mode-set resolution are reused. The committed fixtures are project-owned and
generated locally (AVM is not vendored).
