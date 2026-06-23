## 1. Tracking

- [x] 1.1 Add `DECODE-GENERAL-INTRA-GRID-SMOOTH-H-ABOVE-RIGHT` to the implementation matrix.
- [x] 1.2 Add the `general-intra-grid-smooth-h-above-right` decoder support row.
- [x] 1.3 Add the `syn-shgrid-intra-128x128-q80.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Implementation

- [x] 2.1 Lift the first-superblock-row gate: admit a full-superblock §7.13.2.13 `SMOOTH_H_PRED` luma block at any 2-D grid position (collapse the first-row-admit / row>0-reject arms into one `n4w == FULL_SB_N4_LUMA` admit).
- [x] 2.2 Keep the reconstruction unchanged (the plane-general `reconstruct_general_intra_smooth_over_edges_into` derives `num4AboveRight` over the §5.20.2.3 `BlockDecoded` state and resolves the real §7.13.2.1 `AboveRow[w]`).
- [x] 2.3 Keep the SMOOTH_H SPLIT-child cross-superblock above-right (`general_intra_smooth_h_above_right_unverified`), SMOOTH_V below-left sub-block, SMOOTH chroma sub-block (`general_intra_smooth_chroma_subblock`), and directional-neighbour rejections intact.

## 3. Documentation And Verification

- [x] 3.1 Regenerate feature/status/support docs.
- [x] 3.2 Validate OpenSpec, feature tracking, decoder support, reference evidence, and the Rust acceptance gate.
