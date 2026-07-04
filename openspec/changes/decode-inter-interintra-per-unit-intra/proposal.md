## Why

Coded frame 2 of the local decoder mission stream defers at its first WARPMV
interintra block, and behind it every intra-in-inter block whose transform
partition splits perpendicular to the prediction source edge either
deferred or (before b02's closure) reconstructed confident-wrong: the
decoder predicted once per block where § 5.20.7.24 demands one prediction
per transform unit over just-reconstructed sibling samples. Separately,
the block mode context was derived before reference selection with a
placeholder first reference, diverging from the § 5.20.7.6 order.

## What Changes

- Derive the block mode context after `read_ref_frames` per § 5.20.7.6:
  the single-reference path recomputes it for the selected reference, and
  the compound reader is split into a reference-pair stage and a mode
  stage so the pair context (including the previously dead compound
  neighbour-match arm) feeds the mode and DRL reads.
- Implement § 7.13.3.29 / § 7.13.3.30 smooth-mask interintra prediction
  for WARPMV blocks: the `Ii_Weights_1d` table and per-plane mask/blend
  land in `splot-recon`; the decode path predicts II_DC (with the
  § 7.13.2.12 IBP DC modifier) / II_V / II_H per plane before motion
  compensation and blends after. Wedge interintra, II_SMOOTH, and the
  SIMPLE-path tail stay fail-closed defers after the bit-exact parse.
- Replace the transform-partition predict-once shortcut with the
  § 5.20.7.24 per-transform-unit prediction loop: per-unit re-scoped
  plans over the single-rect reconstruction arms, `BlockDecoded` marked
  per unit, per-unit above-MRL read offsets, and the previously missing
  IBP-DC arm on the intra-in-inter LUMA DC path. The chroma DC arm on the
  same path still omits the § 7.13.2.12 modifier for non-CfL chroma
  (unreachable on any known stream today; named follow-up for the batch
  that first admits non-CfL chroma DC intra-in-inter output).

## Impact

- Affected specs: decoder-support (DECODE-FIRST-INTER-FRAME-FRONTIER)
- Affected code: `splot-recon` (new `workspace_interintra`),
  `splot-decode` inter block path, residual pipeline, partition walk
