## 1. Tracking

- [x] 1.1 Add `DECODE-INTER-SUBPEL-MV` to the implementation matrix.
- [x] 1.2 Add the decoder support row for `inter-subpel-mv`.
- [x] 1.3 Add the `syn-2frame-subpel-inter-64x64.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Fixture verification

- [x] 2.1 Generate `syn-2frame-subpel-inter-64x64.ivf` locally from a project-owned horizontal half-cosine Y4M (frame 1 shifted right by half a sample) with broad decode tools (incl. GDF, intra-dip, bawp, cwp, flex-mvres, adaptive-mvd, global-motion, warp, tip, refinemv, opfl-refine, masked/interintra compound, joint-mvd, ref-frame-mvs) disabled and `--qp 180 --sb-size 64 --min/max-partition-size 64`.
- [x] 2.2 Confirm via `splot inspect` the OBU shape: frame 0 = TD + SEQUENCE_HEADER + CLOSED_LOOP_KEY, frame 1 = TD + REGULAR_TILE_GROUP.
- [x] 2.3 Confirm `avmdec --rawvideo --i420` equals `dav2d --demuxer ivf` byte-for-byte (decoded-output md5 `a0e82de3a95bb4b519c4c84ffa2ba816`, 12288 bytes) and that frame 1 is a fractional shift of frame 0 (a real sub-pel MV, not a copy).
- [x] 2.4 Confirm the fixture validates clean.
- [x] 2.5 Confirm the header facts via parse: interpolation_filter == SWITCHABLE, MvPrecision == EighthPel, motion modes all disabled, NumTotalRefs == 1, skip_mode_present == 0.

## 3. CDF banks

- [x] 3.1 Add the AV2 §9.3 SHELL-coded MV CDF banks (joint_shell_set, the EighthPel P==6 shell_class pair, joint_shell_last_two_classes, shell_offset_low_class / class2 / other_class, col_mv_greater, col_mv_index) and the interp_filter CDF to the tile/block CDF subset, selected per §8.3.2.

## 4. Read MV + interp filter

- [x] 4.1 Implement the §5.20.7.20 SHELL-coded `read_mv()` magnitude path (shell_set, shell_class, the offset derivations, the col split with the §4.11.13 NS(n) remainder) returning the unsigned `(row, col)` magnitudes.
- [x] 4.2 Implement the §5.20.7.13 explicit `mv_sign` pass (one L(1) bypass bit per nonzero component; sign derivation is disabled for EighthPel) and the `mv_clamp_to_integer`, over the zero no-neighbour predictor.
- [x] 4.3 Admit single_mode == NEWMV in the inter block decode, read its DRL + shell MV, and read the §5.20.7.6 interp_filter symbol when SWITCHABLE and needs_interp_filter() is 1 (ctx 3 for the no-neighbour single-ref block).

## 5. MV scaling + motion compensation

- [x] 5.1 Implement the §7.13.3.17 motion-vector scaling (startX/startY with the eighth-pel fractional phase; stepX == stepY == 1024 for the identity scale) and the §7.13.3.18 firstX/firstY/lastX/lastY clip bounds per plane (luma + 4:2:0 chroma via the (2*mv)>>sub adjustment).
- [x] 5.2 Feed a packed `ReferencePlaneView` and the derived `SubpelPredictParams` to `splot_recon::subpel_predict_block` for every plane and write the predicted block into the workspace.

## 6. Gate + honest subset

- [x] 6.1 Relax `validate_inter_frame_core` to admit a SWITCHABLE frame interpolation filter while rejecting enable_flex_mvres / enable_adaptive_mvd, keeping every assumed-absent header/mode fact rejected.
- [x] 6.2 Keep the block decode rejecting residual (skip=0), compound, multi-reference, motion modes, OBMC, warp, BAWP, and CWP before any output.

## 7. Verification

- [x] 7.1 `splot decode --output-format raw` on `syn-2frame-subpel-inter-64x64.ivf` reproduces the whole-stream md5 `a0e82de3a95bb4b519c4c84ffa2ba816` byte-for-byte vs avmdec == dav2d, pinned by `subpel_fixture_per_frame_hash_is_stable`.
- [x] 7.2 The zero-MV inter (4e1bd39f) and the general-intra fixtures still decode byte-identical (no regression).
- [x] 7.3 `cargo xtask ci` passes; `openspec validate --all --no-interactive` passes.
