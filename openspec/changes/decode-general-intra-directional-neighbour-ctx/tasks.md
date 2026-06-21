## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-DIRECTIONAL-NEIGHBOUR-CTX` to the implementation matrix.
- [x] 1.2 Add the `general-intra-directional-neighbour-ctx` decoder support row.
- [x] 1.3 Add the `syn-dirneigh-intra-128x64-q80.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Remove the `if mode_ctx != 0 { return Err(UnsupportedYModeIndexContext) }` guard in `decode_general_intra_block_modes`; read `y_mode_set` / `y_mode_index` from the real § 8.3.2 `TileYModeIndexCdf[ctx]` row (ctx 1 or 2).
- [x] 2.2 Defer the `y_mode_offset` escape over a directional neighbour: reject with `GeneralIntraBlockModeError::UnsupportedDirectionalNeighbourReorder` (runtime `general_intra_directional_neighbour_reorder`) AFTER consuming the `y_mode_offset` symbol, because `reconstruct_y_mode_offset_escape_top_left` only models the no-directional-neighbour § 5.20.5.3 reorder.
- [x] 2.3 Keep the non-directional `get_intra_y_mode_set` short-circuit (`modeIdx < NON_DIRECTIONAL_MODES_COUNT` returns `modeIdx`) for the neighbour-reading DC / SMOOTH(_V) block, and keep the existing directional neighbour-reading luma rejection (`general_intra_multiblock_directional_luma`).

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
