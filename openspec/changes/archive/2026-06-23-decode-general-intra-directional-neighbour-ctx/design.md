## Context

The multi-block neighbour SMOOTH brick (`DECODE-GENERAL-INTRA-MB-NEIGHBOUR-SMOOTH`)
introduced the per-MI `IntraJointModes` grid (`TileIntraJointModeState`) and the
AV2 § 8.3.2 `y_mode_index` context derivation
(`ctx = (get_joint_mode(0) >= NON_DIRECTIONAL_MODES_COUNT) + (get_joint_mode(1) >= NON_DIRECTIONAL_MODES_COUNT)`),
but — lacking a fixture that reached a non-zero context — it REJECTED `ctx != 0`
before reading any `y_mode_set` / `y_mode_index` symbol
(`general_intra_directional_neighbour_y_mode_index_ctx`). The CDF banks
(`TileYModeIndexCdf` / `TileYModeOffsetCdf`) were already indexed by ctx.

`DECODE-GENERAL-INTRA-DIRECTIONAL-FOLLOW-CHROMA` later let a D135 block decode past
its `uv_mode == 0` directional-follow chroma. Combined with the multi-superblock
raster walk, a 128x64 frame whose LEFT 64x64 codes as D135 (storing the directional
`IntraJointMode 36`) and whose RIGHT 64x64 reads that neighbour is now decodable up
to the neighbour-reading block — its § 8.3.2 `y_mode_index` ctx is 1, the exact
case the old code rejected.

## Decisions

- **Read with the real ctx; don't reject.** The only required change for a
  non-directional neighbour-reading block is to delete the `if mode_ctx != 0 {
  return Err(...) }` guard in `decode_general_intra_block_modes`. The `y_mode_set`
  / `y_mode_index` / `y_mode_offset` reads already pass `mode_ctx` to their CDF
  selectors, so the ctx-1 / ctx-2 banks are selected automatically.

- **The § 5.20.5.3 reorder is a no-op for the verified subset.**
  `get_intra_y_mode_set(modeIdx)` returns `modeIdx` unchanged when
  `modeIdx < NON_DIRECTIONAL_MODES_COUNT` (`05-syntax-structures.md` lines
  11116-11118); the directional-neighbour selection loop (lines 11120-11176, which
  preselects the neighbours' joint modes and their `Block_Width * Block_Height > 64`
  ±1..4 expansion ahead of `Default_Mode_List_Y`) is entered ONLY for
  `modeIdx >= NON_DIRECTIONAL_MODES_COUNT`. The verified right block decodes
  `y_mode_set == 0`, `y_mode_index == 2 < NON_DIRECTIONAL_MODES_COUNT` (not the
  `MODE_INDEX_COUNT - 1` escape), so `modeIdx == 2` maps straight through to
  `Reordered_Y_Mode[2] == SMOOTH_V_PRED` via the existing
  `reconstruct_minimal_y_mode`. No new reorder code is needed for this brick.

- **Defer the `y_mode_offset` escape over a directional neighbour.** The escape
  (`y_mode_set == 0`, `y_mode_index == MODE_INDEX_COUNT - 1`) has
  `modeIdx = (MODE_INDEX_COUNT - 1) + y_mode_offset >= NON_DIRECTIONAL_MODES_COUNT`,
  so over a directional neighbour `get_intra_y_mode_set` WOULD enter the reorder
  loop. `reconstruct_y_mode_offset_escape_top_left` only models the
  no-directional-neighbour case (both `get_joint_mode` out of frame /
  non-directional, `count == 0`), and the resolved mode would be directional
  (needing the deferred § 7.13.2.8 luma IDIF). So when the escape is taken with
  `mode_ctx != 0` the decode rejects with
  `GeneralIntraBlockModeError::UnsupportedDirectionalNeighbourReorder` AFTER the
  `y_mode_offset` symbol is consumed (runtime `general_intra_directional_neighbour_reorder`).

- **Keep the verified-subset admission discipline.** The runtime still rejects a
  directional neighbour-reading LUMA block over a real non-flat edge
  (`general_intra_multiblock_directional_luma`, needs the § 7.13.2.8 IDIF), so a
  confident decode stays bit-exact.

## Verification

- The committed `syn-dirneigh-intra-128x64-q80.ivf` (LEFT D135 luma via the
  `y_mode_offset` escape `IntraJointMode 36`, RIGHT SMOOTH_V luma, flat chroma)
  decodes bit-exactly to the avmdec AND dav2d oracle (raw md5
  `1a84b6545ee333b98cdf1982fd18310a`, pinned splot frame hash
  `ad1515885df5620a31c37f855934ae2432167edbf1b1b62081552b9df3957426`), guarded by
  § 8.2.4 `exit_symbol()`. Temporary instrumentation (since removed) confirmed the
  RIGHT block's § 8.3.2 ctx is 1.
- All existing general-intra fixtures still decode bit-exact; a directional
  neighbour-reading luma block (needing the § 7.13.2.8 IDIF) still rejects.
