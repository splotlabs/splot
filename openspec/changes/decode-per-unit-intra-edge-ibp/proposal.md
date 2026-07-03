# Decode per-unit intra edge filters and IBP blend

## Why

Intra-coded transform units inside inter frames reconstructed with no-op
§ 7.13.2.7 edge filters and without the § 7.13.2.9 IBP second
directional predictor: the per-unit residual arms passed default filter
state, square single-unit directional plans bypassed the rect arms
entirely, the § 5.20.7.29 WAIP wide-angle remap ran once per block with
block dimensions where the reference decoder remaps per transform unit,
and a fractional block vector reached an IntraBC placeholder that
self-copied the zero-initialized target. Coded frame 2 of the mission
stream diverged from the AVM oracle on 730,992 pre-filter luma samples.

## What Changes

- A tile-wide `YModes`-smoothness grid records each decoded block's luma
  smoothness (inter blocks record `false`, matching the spec's
  inter-mode `YModes` entries); the per-unit § 7.13.2.15/16 filter types
  read the unit's above/left cells.
- The one-sided and middle luma arms resolve real § 7.13.2.17 strengths,
  § 7.13.2.14 corners, and `numPx` clamps; the even-angle-delta
  one-sided arms blend the opposite-edge § 7.13.2.8 prediction per
  § 7.13.2.9 (`applyIbp` on the TU size, `MrlIndex == 0`, luma only).
- Every mrl-0 directional unit re-derives its `pAngle` through the
  § 5.20.7.29 WAIP remap with the UNIT's dimensions and re-selects its
  zone arm; square single-unit directional plans route through the same
  rect arms instead of bypassing the machinery.
- Fractional-vector IntraBC predictions run the § 7.13.3.18 separable
  convolution with the 2-tap BILINEAR row over the current frame
  (07:7824-7828 frame clipping) instead of self-copying.
- A diagnostics-only `SPLOT_DUMP_PREFILTER` dump writes each frame's
  pre-filter workspace for oracle diffing (env-gated, inert by default).

## Impact

- Affected specs: decoder-support (DECODE-FIRST-INTER-FRAME-FRONTIER)
- Affected code: `splot-decode` residual pipeline, general-intra plan
  derivation, inter walk state, IntraBC prediction, plus the shared
  wienerns_lr visibility bumps
- Disclosed remaining divergence on the mission stream: pre-filter
  chroma (chroma edge filters / IBP-DC chroma / chroma IntraBC) and the
  § 7.17 deblock geometry hand-off on inter frames (per-unit transform
  edges, coding-block origins, sub-PU edges) — both diagnosed and owned
  by named follow-ups.
