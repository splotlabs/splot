## Why

The general intra decode path reconstructs DC_PRED blocks and the first non-DC
luma modes (§ 7.13.2.13 `SMOOTH_V`/`SMOOTH_H`) bit-exactly. Real AV2 intra frames
also use the directional-angle modes, which are coded through the § 5.20.5.3
`y_mode_offset` escape and predicted by the § 7.13.2.8 single directional
prediction process. The next step is the first directional-angle luma mode:
`D135_PRED` (pAngle 135), for the top-left no-neighbour block where the
§ 7.13.2.8 prediction edges reduce to the § 7.13.2.1 flat fallbacks.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-ANGLE`.
- Reconstruct the § 5.20.5.3 `y_mode_offset` escape (`y_mode_set == 0`,
  `y_mode_index == MODE_INDEX_COUNT - 1`): add the `TileYModeOffsetCdf` selector
  (sharing the `y_mode_index` § 8.3.2 context, defaults from
  `DEFAULT_Y_MODE_OFFSET_CDF`), read `y_mode_offset`, and resolve the typed
  `YMode` + `AngleDeltaY` via `reconstruct_y_mode_offset_escape_top_left`
  (`get_intra_y_mode_set` over `Default_Mode_List_Y` for the top-left
  no-directional-neighbour case, then the `Reordered_Y_Mode` directional reorder).
- Generalize the chroma resolver to the directional luma branch of § 5.20.5.3
  `get_intra_uv_mode_set` (`modeIdx == 0 -> YMode`, then the `mode != YMode`
  filter), keeping the supported chroma subset DC / SMOOTH.
- Reconstruct § 7.13.2.8 `D135_PRED` for the top-left (no-neighbour) block: build
  the prediction over the § 7.13.2.1 no-neighbour fallback edges (8-bit:
  `AboveRow` `127`, `LeftCol` `129`, shared corner `128`) via the shared
  `splot-recon` `predict_intra_middle_directional_angle_rect_into`, then add the
  § 5.20.7.27 residual.
- Gate the directional block decode to the verified subset (top-left 64x64
  superblock, pAngle 135, `AngleDeltaY == 0`, DC chroma); reject everything else
  with a structured `decode/unsupported-feature` diagnostic before reconstruction.
- Add the project-owned `syn-hedge-intra-64x64-q80.ivf` fixture and prove it
  decodes bit-exactly to the avmdec/dav2d oracle.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-angle`: Crate-private single-block directional-angle
  (§ 7.13.2.8 `D135_PRED`, pAngle 135) luma intra prediction over the § 7.13.2.1
  no-neighbour fallback edges plus residual, including the § 5.20.5.3
  `y_mode_offset` escape reconstruction.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra single-block directional-angle luma decode.

## Impact

- Affects `crates/splot-decode/src/tile_payload/cdf/block_context.rs`,
  `crates/splot-decode/src/tile_payload/cdf/block_rows.rs`,
  `crates/splot-decode/src/tile_payload/cdf.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_block.rs`,
  `crates/splot-decode/src/tile_payload.rs`,
  `crates/splot-decode/src/runtime_minimal.rs`, and
  `crates/splot-decode/src/runtime_minimal_recon.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`, and generated status docs.
- No public API, dependency graph, encoder, or validator changes. Non-zero angle
  deltas, the other directional modes, sub-64x64 directional blocks (mode-dependent
  non-DCT TxType), neighbour-having directional blocks (full § 7.13.2.8 IDIF edge
  synthesis), directional chroma, non-64x64 frames, inter prediction, in-loop
  filters, and live in-CI AVM/dav2d remain out of scope.
