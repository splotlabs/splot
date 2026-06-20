## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-MB-NEIGHBOUR-SMOOTH` to the implementation matrix.
- [x] 1.2 Add the `general-intra-mb-neighbour-smooth` decoder support row.
- [x] 1.3 Add the `syn-mbvg-intra-128x64-q80.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Confirm the decoded luma mode of each superblock via temporary instrumentation (both SMOOTH_V_PRED; right superblock has a left neighbour), then remove the instrumentation.
- [x] 2.2 Generalize the §7.13.2.1 smooth edge builder from chroma-only (`build_smooth_chroma_edges`) to plane-general (`build_smooth_edges`), and generalize the `num4AboveRight` derivation to luma (`full_sb_num4_above_right`, `sub_x == 0`).
- [x] 2.3 Add `reconstruct_general_intra_luma_nondc_neighbour_block_into`: build the §7.13.2.1 `LeftCol`/`AboveRow` edges from the partially-built frame's real reconstructed neighbour, run the shared `predict_intra_smooth_rect_into` SMOOTH_V/H mode, and add the §5.20.7.27 residual.
- [x] 2.4 Relax the multi-block non-DC luma gate to allow a neighbour-having SMOOTH_V/H full 64x64 superblock block; keep rejecting neighbour-having directional (D135) luma, sub-superblock non-DC, and not-yet-verified modes before reconstruction.
- [x] 2.5 Dispatch the neighbour-having SMOOTH_V/H luma block to the new recon function (the no-neighbour top-left path is unchanged).
- [x] 2.6 Add the per-MI `IntraJointModes` grid (`TileIntraJointModeState`) and thread it through the general intra partition walk (`decode_general_intra_multiblock_tree` -> `decode_general_intra_partition_tree` -> the `on_leaf` callback -> `decode_one_general_intra_block`); record each block's reconstructed `IntraJointMode` (`modeDelta`) after the leaf.
- [x] 2.7 Compute the real § 8.3.2 `y_mode_index` context (`get_joint_mode(left) + get_joint_mode(above)`) from the grid in `decode_general_intra_block_modes` before reading any symbol; reject `ctx != 0` with a typed `decode/unsupported-feature` diagnostic and keep `ctx == 0` decoding exactly as before (codex P2 on PR #385 + latent #383 fix).

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
