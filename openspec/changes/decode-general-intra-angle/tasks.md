## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-ANGLE` to the implementation matrix.
- [x] 1.2 Add the `general-intra-angle` decoder support row.
- [x] 1.3 Add the `syn-hedge-intra-64x64-q80.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Add the `TileYModeOffsetCdf` selector (block_rows.rs / cdf.rs) backed by `DEFAULT_Y_MODE_OFFSET_CDF`, sharing the `y_mode_index` §8.3.2 context.
- [x] 2.2 Reconstruct the §5.20.5.3 `y_mode_offset` escape for the top-left no-directional-neighbour case (`reconstruct_y_mode_offset_escape_top_left`: `get_intra_y_mode_set` over `Default_Mode_List_Y`, then the `Reordered_Y_Mode` directional reorder to the typed `YMode`/`AngleDeltaY`).
- [x] 2.3 Generalize the chroma resolver to the directional luma branch of §5.20.5.3 `get_intra_uv_mode_set` (`supported_chroma_mode`), keeping the DC / SMOOTH supported subset.
- [x] 2.4 Add `reconstruct_general_intra_luma_directional_first_block_into`: build the §7.13.2.8 prediction over the §7.13.2.1 no-neighbour fallback edges via `predict_intra_middle_directional_angle_rect_into` and add the §5.20.7.27 residual.
- [x] 2.5 Gate the directional block decode to the verified subset (top-left 64x64 superblock, pAngle 135, `AngleDeltaY == 0`, DC chroma); reject everything else before reconstruction.

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
