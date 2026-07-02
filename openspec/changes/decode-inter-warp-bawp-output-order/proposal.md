## Why

Coded frame 2 of the `ac0ej3` mission stream deferred at its first
EXTENDWARP block, and behind it sat the LOCALWARP half of the same warp
dependency chain, a BAWP block, small-block/invalid-shear warp geometry,
and the remaining per-unit intra arms. Independently, the decoder
emitted output frames in decode order where § 7.21 requires
display-order release, and the MV stack lacked the § 7.12.2.21
reference MV bank — the dominant mechanism behind frame 2's
reconstruction divergence.

## What Changes

- Retain warp neighbour state (motion mode, warp model, block geometry,
  second-list MV, MV-stack offsets, extend-base deltas) and derive
  EXTENDWARP models per § 7.13.3.24 and LOCALWARP models per § 7.12.3 +
  § 7.13.3.23 over the shared § 5.20.7.13 tail; warp blocks record a
  non-NEWMV neighbour mode per § 7.11.3.
- Predict invalid-shear and sub-8x8 warp geometry with the § 7.13.3.20
  extended block warp instead of erroring (§ 7.13.3.15 skipPred).
- Implement § 7.13.3.25 block adaptive weighted prediction (implicit
  template fit and explicit scales, chroma reuse of the luma alpha).
- Complete the per-unit intra arms: kernel-identical square-to-rect
  mappings for the directional and smooth plans, and § 5.20.7.24
  `allowCorners = 0` counts on middle units.
- Output frames in display order per § 7.21 (held implicit-output
  frames, immediate/refresh/successive/end-of-stream release).
- Implement the § 7.12.2.21 reference MV bank with the § 5.20.2.2
  per-superblock reset and re-seed.
- Read § 5.18.2 `tip_frame_mode`; mode 1 (TIP frames) stays fail-closed.

## Impact

- Affected specs: decoder-support (DECODE-FIRST-INTER-FRAME-FRONTIER)
- Affected code: `splot-decode` inter path, `splot-recon` warp/BAWP
  kernels, `splot-core` inter frame-header tail
