# Decode inter warp reference list

## Why

Coded frame 2 of the `ac0ej3` mission stream parsed end-to-end after the
b04 batch but reconstructed 41% dirty: both WARPMV consumers used an
identity warp base because the § 7.12.2 Warp Reference List
(`WarpParamStack`) and the § 5.20.2.2 warp parameter bank did not exist,
warp blocks recorded their block MV into every grid cell where the
§ 7.12.2.12 scan expects per-8x8 `SubMvs` projections, and blocks wider
and taller than 32 lacked the § 7.12.2.20 mixture candidates their DRL
indices select.

## What Changes

- Build the § 7.12.2 `WarpParamStack` inside `find_mv_stack` under
  `DeriveWrl`: the § 7.12.2.3/.4 corner-derived model (both iterations;
  warp corners project via `get_warp_motion_vector_xy_pos`), the
  § 7.12.2.9 spatial inserts fired from every scan point, then the
  § 7.12.2.20 tail — warp bank newest-first, `gm_params` (identity while
  global-motion frames defer at the frame gate), and two identity
  defaults — with the § 7.12.2.11 four-slot no-dedup insert.
- Implement the § 5.20.2.2 warp parameter bank on the MV bank's
  superblock lifecycle (contents per superblock row, hits and list-0
  re-seeding per superblock) through shared seed-walk and bank-ring
  helpers; every warp block updates it unconditionally per § 5.20.7.
- Consume the stack: WARPMV projects `WarpParamStack[RefWarpIdx]`
  through § 7.12.2.2 for its predicted MV and takes its parameters from
  the stack entry; WARP_NEWMV DELTAWARP bases read the same stack,
  retiring the `ref_warp_idx != 0` defer and the identity-base bypasses.
- Record § 7.13.3.20 `SubMvs` per covered MI cell for warp blocks (the
  8x8-unit center projection) and read them from the § 7.12.2.12
  `get_mv` consumers; the banks and § 7.12.3 keep the block MV.
- Add the § 7.12.2.20 mixture candidates for blocks wider and taller
  than 32, budget-deduped under the shared `PruneCount`.
- Apply the § 7.12.2.19 strict-max-weight nearest reorder when the
  sequence selects it (`DrlReorder`), suppressed per § 7.12.2 when
  `useTemporalFirst` holds for the block.
- Gate the § 7.14.4 TCQ `dqDenom` extra shift on `TX_CLASS_2D`
  transform classes.
- Add a diagnostics-only `SPLOT_DUMP_CODED_FRAMES` decode-order frame
  dump (the § 7.21 scheduler removed decode-order output).

## Impact

- Affected specs: decoder-support (DECODE-FIRST-INTER-FRAME-FRONTIER)
- Affected code: `splot-decode` inter MV-stack/bank and warp syntax
  paths, plus the diagnostics dump in the minimal runtime driver
