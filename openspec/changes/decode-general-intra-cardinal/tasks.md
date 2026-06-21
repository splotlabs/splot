## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-CARDINAL` to the implementation matrix.
- [x] 1.2 Add the `general-intra-cardinal` decoder support row.
- [x] 1.3 Add the `syn-vpred-intra-64x128-q160.ivf` and `syn-hpred-intra-128x64-q180.ivf` fixtures, conformance manifest entries, and reciprocal LOCAL-REFERENCE-EVIDENCE entries.

## 2. Implementation

- [x] 2.1 Reconstruct the § 5.20.5.3 typed `YMode` for the DIRECT first-mode-set directional `y_mode_index` (`NON_DIRECTIONAL_MODES_COUNT <= y_mode_index < MODE_INDEX_COUNT - 1`) at ctx == 0 via the shared `resolve_y_mode_top_left` / `get_intra_y_mode_set_top_left`; reject ctx != 0 (the unmodelled § 5.20.5.3 directional-neighbour reorder). Map `V_PRED` / `H_PRED` to `SupportedDirectionalLumaMode::Vertical` / `Horizontal` and the `uv_mode == 0` follow chroma to `SupportedChromaMode::VerticalFollow` / `HorizontalFollow`.
- [x] 2.2 Add `reconstruct_general_intra_cardinal_neighbour_block_into` (read the real § 7.13.2.1 above row / left column from `intra_dc_edges_for_rect` and run `predict_intra_cardinal_directional_rect_into`); admit a row>0 full-superblock V_PRED luma block and a non-first-column full-superblock H_PRED luma block and dispatch them; keep first-row V / first-column H / sub-superblock / ctx != 0 / non-cardinal blocks rejected.
- [x] 2.3 Admit the `uv_mode == 0` directional-follow V_PRED / H_PRED chroma for the same neighbour-having block (routing `VerticalFollow` / `HorizontalFollow` to the cardinal recon); keep first-row/first-column chroma rejected.

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
