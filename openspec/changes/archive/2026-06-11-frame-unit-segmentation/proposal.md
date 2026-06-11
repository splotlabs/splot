# Proposal: coded-frame-unit segmentation (§ 7.3.3–§ 7.3.5, sound subset)

## Feature IDs

- `AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT`, `AV2-7.3.4-CODED-NONOUTPUT-FRAME-UNIT`,
  `AV2-7.3.5-CODED-FRAME-UNIT` (todo → partial/done per what lands)
- `AV2-7.3.7-TEMPORAL-UNIT-ORDER` (the two planned backlog rows)
- `AV2-5.13-QUANTIZATION-MATRIX`, `AV2-5.14-FILM-GRAIN` (window-reset upgrades)
- `AV2-5.17-METADATA` family (§ 6.16.5/§ 6.16.6 halves, lifetime upgrade)

## Why

The three § 7.3.3–§ 7.3.5 rows are the matrix's largest `validate = todo`
cluster and the named blocker for a dozen other residuals. A sound
segmentation model is buildable now from already-parsed state: the
(xlayer, mlayer, tlayer) triple, OBU types, `metadata_is_suffix`,
`is_first_tile_group` (§ 5.19 prefix), and `immediate/implicit_output_frame`
from the core frame-header parser on supported paths — with unsupported
parse stops routing to an explicit Unknown segment that never fires a
diagnostic.

## What Changes

Grounded in `07-decoding-process.md#s-7-3-3`–`#s-7-3-5` (lines 367–510) and
`#s-7-3-8-10` (line ~880):

1. **Segmentation model** (`frame-unit/` namespace): per-triple consecutive
   OBU runs partition into coded frame units with the § 7.3.3/§ 7.3.4
   region order — CI (zero or one) → MFHs → pre-frame region (BRT / QM /
   FGM / prefix metadata, any order; BRT **zero-or-one in non-output
   units**, zero-or-more in output units) → the coded frame (same-type tile
   OBUs with the `is_first_tile_group` first/rest rule, or exactly one
   SEF) → suffix-metadata tail. PADDING is position-free. Output vs
   non-output classification from the parsed output flags; any
   undecidable OBU makes the unit Unknown (no diagnostics).
2. **Presence-order diagnostics** (errors, § 7.3.3/§ 7.3.4): region-order
   violations, duplicate CI in a unit, BRT multiplicity in non-output
   units, prefix-metadata after the coded frame, suffix-metadata before
   it, `frame-unit/first-tile-group-flag` (first OBU shall have
   `is_first_tile_group = 1`, the rest 0), SEF single-OBU rule, mixed
   frame OBU types in one coded frame.
3. **The two planned § 7.3.7 backlog rows** land:
   `obu-order/global-hls-after-metadata-suffix` and
   `obu-order/non-global-hls-before-coded-layer` (roadmap backlog table
   rows; their triggers now decidable from the segmentation).
4. **§ 7.3.8.10**: CI only in the first coded frame unit of its temporal
   unit (per layer); plus the § 6.16.5/§ 6.16.6 "shall be indicated at the
   first coded picture" halves (existing TODOs at context.rs ~3252).
5. **Consumer upgrades**: metadata-lifetime NO_PERSISTENCE expiry at true
   coded-frame-unit granularity (metadata_lifetime.rs ~253 TODO); QM/FGM
   duplicate-window resets at true frame-unit boundaries, removing the
   documented SEF-only false negatives in both matrix notes.
6. **§ 6.5 SeenFrameHeader groundwork**: the segmentation state exposes
   what `AV2-5.5-TEMPORAL-DELIMITER`'s residual needs (note-level; the
   tile-group half stays blocked on § 5.19 completion, named).

## Non-goals

- Frame-header-derived classification for unsupported parse paths
  (Unknown units stay Unknown; the residual narrows as frame parsing
  lands).
- § 7.3.6/§ 7.3.7 DOH and OrderHint constraints (`celu-orderhint-
  constraints`, next item).
- The § 5.19/§ 5.20 tile-group parsing itself.

## Acceptance criteria

- [ ] Every region rule has violation + boundary + Unknown-silence tests;
  both unit kinds; PADDING anywhere; interleaving allowed where the
  mirror says "any order"; the BRT output/non-output asymmetry tested
  both ways.
- [ ] The QM/FGM SEF false-negative removal is regression-tested (the
  documented miss now fires); metadata lifetime expiry at unit
  granularity tested.
- [ ] All established invariants (frame-confirmation, external-HLS policy,
  per-TU attribution, dedup, Unknown-never-fires) applied.
- [ ] Matrix rows advance with proof; registry/feature-status/ci/coverage
  gates pass.
