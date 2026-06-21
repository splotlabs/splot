## Why

The general intra decode reconstructs the § 7.13.2.8 `D135_PRED` (pAngle 135)
directional luma mode (no-neighbour and neighbour-having, the "middle" angle), but
NOT the two CARDINAL directional modes `V_PRED` (pAngle 90) and `H_PRED`
(pAngle 180). A block whose luma resolves to V_PRED / H_PRED was rejected with
`general_intra_unsupported_y_mode` (the typed `YMode` reconstruction did not map
the `y_mode_index` that selects them), and its `uv_mode == 0` directional-follow
chroma with `general_intra_non_dc_chroma_mode`.

The cardinal cases are the simplest § 7.13.2.8 prediction: step 4 is
`pred[i][j] = AboveRow[j]` (V_PRED, a pure vertical copy of the § 7.13.2.1 above
row) and step 5 is `pred[i][j] = LeftCol[i]` (H_PRED, a pure horizontal copy of
the left column). They read ONLY the above row (V) or the left column (H) — no
corner, no IDIF, no edge synthesis, and no `useIBP` (§ 7.13.2.7 gates `useIBP` on
`pAngle < 90 || pAngle > 180`, and its edge-filter step is skipped entirely for
`pAngle == 90 || pAngle == 180`). So, unlike a middle angle, the cardinal copy is
bit-exact over a NON-flat reconstructed edge with no interpolation, and the
`splot-recon` `predict_intra_cardinal_directional_rect_into` primitive already
implements it.

KEY DECODE-PATH FINDING (corrects the original brief's `y_mode_offset` escape
hint): at § 8.3.2 ctx == 0, V_PRED / H_PRED with `AngleDeltaY == 0` are NOT
reachable via the `y_mode_offset` escape (its `modeIdx` range 7..12 only reaches
D45/D67/D113/D135/D157/D203). They are coded via the § 5.20.5.3 DIRECT first-mode-set
`y_mode_index` (`modeIdx == y_mode_index`, no escape): V_PRED at `y_mode_index ==
5` (`get_intra_y_mode_set(5)` → `Default_Mode_List_Y[0] == 17` → `modeDelta 22` →
`Reordered_Y_Mode[7] == V_PRED`), H_PRED at `y_mode_index == 6`
(`Default_Mode_List_Y[1] == 45` → `modeDelta 50` → `Reordered_Y_Mode[11] ==
H_PRED`). This was confirmed empirically: avmenc with `--enable-smooth-intra=0`
over a perfect vertical/horizontal continuation at base_q_idx 160/180 codes
`y_mode_set == 0, y_mode_index == 5/6` (at lower QP it preferred an angled
`y_second_mode` V/H delta). Lifting the reject (verified bit-exact against avmdec
AND dav2d) is the first general-intra cardinal directional decode.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-CARDINAL`.
- Reconstruct the § 5.20.5.3 typed `YMode` for the DIRECT first-mode-set directional
  `y_mode_index` (`NON_DIRECTIONAL_MODES_COUNT <= y_mode_index < MODE_INDEX_COUNT -
  1`) at ctx == 0 via the shared `resolve_y_mode_top_left` /
  `get_intra_y_mode_set_top_left` (the same ctx == 0 selection the `y_mode_offset`
  escape uses); reject ctx != 0 (the unmodelled § 5.20.5.3 directional-neighbour
  reorder).
- Map `V_PRED` / `H_PRED` to `SupportedDirectionalLumaMode::Vertical` /
  `Horizontal`, and the `uv_mode == 0` follow chroma to
  `SupportedChromaMode::VerticalFollow` / `HorizontalFollow`.
- Admit a row>0 full 64x64 superblock (`n4w == 16`) V_PRED luma block (real above
  row) and a non-first-column full 64x64 superblock H_PRED luma block (real left
  column), plus their `uv_mode == 0` follow chroma.
- Add the plane-general `reconstruct_general_intra_cardinal_neighbour_block_into`,
  which reads the real § 7.13.2.1 edge from `intra_dc_edges_for_rect` (above row
  for V, left column for H) and runs `predict_intra_cardinal_directional_rect_into`.
- Keep deferred (still rejected): a first-superblock-row V_PRED / first-column
  H_PRED reading the § 7.13.2.1 no-neighbour fallback
  (`general_intra_cardinal_vertical_unverified` /
  `general_intra_cardinal_horizontal_unverified`), sub-superblock (split) cardinal
  blocks, a directional-neighbour (ctx != 0) cardinal escape/reorder, non-cardinal
  angles, non-zero angle deltas, the `y_second_mode` (`y_mode_set != 0`) path,
  non-64x64 frames, inter prediction, and in-loop filters.
- Add the project-owned `syn-vpred-intra-64x128-q160.ivf` (top DC + bottom V_PRED)
  and `syn-hpred-intra-128x64-q180.ivf` (left DC + right H_PRED) fixtures and prove
  each decodes bit-exactly to the avmdec AND dav2d oracle.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-cardinal`: Crate-private general intra cardinal directional
  (`V_PRED` / `H_PRED`) luma plus directional-follow cardinal chroma decode over a
  real reconstructed § 7.13.2.1 edge.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra cardinal directional decode.

## Impact

- Affects `crates/splot-decode/src/tile_payload/cdf/block_context.rs` (the
  `V_PRED` / `H_PRED` mode mapping, the first-set directional `YMode` reconstruction,
  and the cardinal follow chroma resolution), `crates/splot-decode/src/tile_payload/general_intra_block.rs`
  (the first-set directional mode-decode branch),
  `crates/splot-decode/src/runtime_minimal_recon.rs` (the new
  `reconstruct_general_intra_cardinal_neighbour_block_into`), and
  `crates/splot-decode/src/runtime_minimal/general_intra.rs` (the luma + chroma
  admission gates and dispatch). No new public surface; the recon cardinal
  predictor is reused.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and
  generated status docs.
- No dependency graph, encoder, or validator changes. A first-superblock-row
  V_PRED / first-column H_PRED, a directional-neighbour (ctx != 0) cardinal escape,
  sub-superblock cardinal blocks, other angles / non-zero deltas, the
  `y_second_mode` path, non-64x64 frames, inter prediction, and in-loop filters
  remain out of scope and rejected.
