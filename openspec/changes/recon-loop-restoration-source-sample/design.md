## Context

The local decoder mission decoder frontier now reaches supported frame-level Wiener NS unit
selection state and rejects because active loop-restoration reconstruction is not
implemented. AV2 section 7.20.2 is a small source-sample process used by loop
restoration filters: it clips requested coordinates to the allowed luma-derived
filtering extent, chooses `CdefFrame` for samples inside the current restoration
stripe, and chooses `CurrFrame` for samples outside the stripe while clamping to
at most two lines above or below it.

## Goals / Non-Goals

**Goals:**

- Add a scheduler-free, panic-free `splot-recon` selector for AV2 section 7.20.2.
- Return clipped plane coordinates and a `CurrFrame` / `CdefFrame` source enum
  instead of reading frame storage directly.
- Keep luma/chroma subsampling, luma extent validation, stripe bounds, and
  out-of-stripe two-line clamping source-backed and tested.

**Non-Goals:**

- Full section 7.20 loop-restoration traversal, frame storage reads, Wiener NS
  filter application, PC-Wiener classification, chroma Wiener NS filtering, GDF,
  BRU, runtime decode wiring, or local decoder mission output.

## Decisions

- **Caller-resolved luma bounds.** The helper receives the luma `LumaStartX`,
  `LumaEndX`, `LumaStartY`, `LumaEndY`, `LumaStripeStartY`, and
  `LumaStripeEndY` values derived by section 7.20.1. That keeps tile boundary,
  stripe, restoration unit, and frame-dimension policy outside this pure helper.
- **Source enum, not source callback.** Section 7.20.2 returns a frame sample,
  but `splot-recon` should not own frame staging yet. Returning the selected
  source and clipped coordinates lets later runtime wiring read from
  `CurrFrame`, `CdefFrame`, or test doubles without changing the selector.
- **Validate caller facts up front.** The bounds type rejects inverted luma
  ranges, stripe ranges outside the luma extent, and subsampling values outside
  the AV2 `0..=1` domain before any coordinate resolution.
- **Saturating two-line clamp.** The spec uses `stripeStartY - 2` and
  `stripeEndY + 2`; the helper uses saturating arithmetic for those intermediate
  limits after the already-clipped coordinate comparison. This keeps the helper
  total for extreme caller-supplied bounds while preserving normal AV2 frame
  behavior.

## Risks / Trade-offs

- This brick does not unblock local decoder mission by itself. Runtime code must still compose
  loop-restoration traversal, frame-level filter-bank selection, source reads,
  Wiener NS filtering, and output storage before the active LR diagnostic can
  move.
- The caller can supply incorrect section 7.20.1 luma bounds. The helper validates
  internal consistency but intentionally does not derive tile/frame extents.
  Matrix notes keep that boundary explicit for later runtime integration.
