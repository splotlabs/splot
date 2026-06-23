## Why

The general intra `y_mode_index` decode derives the AV2 § 8.3.2 context from the
per-MI `IntraJointModes` neighbour grid (`DECODE-GENERAL-INTRA-MB-NEIGHBOUR-SMOOTH`),
but it REJECTS a non-zero context (`general_intra_directional_neighbour_y_mode_index_ctx`):
a block whose already-decoded left/above neighbour stored a directional
`IntraJointMode` (`>= NON_DIRECTIONAL_MODES_COUNT`) could not be decoded because no
oracle fixture reached the `ctx != 0` CDF row. `DECODE-GENERAL-INTRA-DIRECTIONAL-FOLLOW-CHROMA`
now lets a D135 block decode past its `uv_mode == 0` chroma, so a multi-block frame
with a D135 left neighbour is finally decodable up to the neighbour-reading block —
making the `ctx != 0` `y_mode_index` read verifiable against the AVM/dav2d oracle.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-DIRECTIONAL-NEIGHBOUR-CTX`.
- Lift the `ctx != 0` reject in `decode_general_intra_block_modes`: read
  `y_mode_set` / `y_mode_index` from the real § 8.3.2 `TileYModeIndexCdf[ctx]` row
  (ctx 1 or 2 from the per-MI `IntraJointModes` grid, already computed) instead of
  rejecting before the read. The CDF banks are already indexed by ctx.
- Keep the § 5.20.5.3 luma reconstruction faithful: `get_intra_y_mode_set` returns
  `modeIdx` unchanged for `modeIdx < NON_DIRECTIONAL_MODES_COUNT`
  (`05-syntax-structures.md` lines 11116-11118), so a neighbour-reading block that
  decodes a non-directional `y_mode_index` (DC / SMOOTH(_V), `y_mode_index < 5`, not
  the escape) maps directly through `reconstruct_minimal_y_mode` — the
  directional-neighbour reorder loop never runs.
- Defer the § 5.20.5.3 in-frame directional-neighbour reorder of
  `get_intra_y_mode_set` (the joint-mode selection at lines 11120-11176), reachable
  only via the `y_mode_offset` escape (`y_mode_index == MODE_INDEX_COUNT - 1`, whose
  `modeIdx >= NON_DIRECTIONAL_MODES_COUNT` enters the loop). Over a directional
  neighbour that escape is rejected AFTER consuming the `y_mode_offset` symbol (new
  `GeneralIntraBlockModeError::UnsupportedDirectionalNeighbourReorder`, runtime
  diagnostic `general_intra_directional_neighbour_reorder`), since the resolved mode
  would be directional and need the deferred § 7.13.2.8 luma IDIF.
- Add the project-owned `syn-dirneigh-intra-128x64-q80.ivf` fixture (LEFT 64x64
  D135 luma, RIGHT 64x64 SMOOTH_V luma whose § 8.3.2 ctx is 1 from the D135 left
  neighbour) and prove it decodes bit-exactly to the avmdec AND dav2d oracle, where
  the old code rejected the frame.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-directional-neighbour-ctx`: Crate-private general intra
  directional-neighbour (`ctx != 0`) `y_mode_index` decode for a neighbour-reading
  block resolving to a non-directional (DC / SMOOTH(_V)) luma mode.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra directional-neighbour `y_mode_index` decode.

## Impact

- Affects `crates/splot-decode/src/tile_payload/general_intra_block.rs` (removes the
  `ctx != 0` guard, adds the `y_mode_offset`-escape directional-neighbour reorder
  deferral) and the runtime error mapping + a stale comment in
  `crates/splot-decode/src/runtime_minimal.rs`. No new public surface.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and
  generated status docs.
- No dependency graph, encoder, or validator changes. The § 5.20.5.3 in-frame
  directional-neighbour reorder, directional neighbour-reading luma over a real
  non-flat edge (needs the § 7.13.2.8 IDIF), the `y_mode_set != 0` / `y_second_mode`
  path, sub-superblock non-DC blocks, multiple tiles, inter prediction, and in-loop
  filters remain out of scope.
