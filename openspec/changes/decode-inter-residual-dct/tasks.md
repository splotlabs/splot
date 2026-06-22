## 1. Tracking

- [x] 1.1 Add `DECODE-INTER-RESIDUAL-DCT` to the implementation matrix.
- [x] 1.2 Add the decoder support row for `inter-residual-dct`.
- [x] 1.3 Add the `syn-2frame-inter-residual-64x64.ivf` fixture, conformance manifest entry, and reciprocal LOCAL-REFERENCE-EVIDENCE entry.

## 2. Fixture verification

- [x] 2.1 Generate `syn-2frame-inter-residual-64x64.ivf` locally from a project-owned flat-100 luma + flat chroma Y4M (frame 1 = frame 0 plus a smooth low-frequency luma delta) with broad decode tools (incl. GDF, intra-dip, intra-edge-filter, bawp, cwp, flex-mvres, adaptive-mvd, warp, tip, refinemv, opfl-refine, masked/wedge/interintra/onesided/diff-wtd compound, joint-mvd, ref-frame-mvs, inter-ist, inter-ddt, cctx, fsc, idtx-intra) disabled and `--enable-global-motion=1 --qp 80 --sb-size 64 --min/max-partition-size 64`.
- [x] 2.2 Confirm via `splot inspect` the OBU shape: frame 0 = TD + SEQUENCE_HEADER + CLOSED_LOOP_KEY, frame 1 = TD + REGULAR_TILE_GROUP, and that frame 1's inter block decodes skip == 0.
- [x] 2.3 Confirm `avmdec --rawvideo --i420` equals `dav2d --demuxer ivf` byte-for-byte (decoded-output md5 `ab2b067aed48cf46035fa031cefb3ab1`, 12288 bytes) and that frame 1's luma differs from frame 0 (a real coded residual, not a copy).
- [x] 2.4 Confirm the fixture validates clean.
- [x] 2.5 Confirm the inter header facts: NumTotalRefs == 1, skip_mode_present == 0, motion modes all disabled, enable_flex_mvres / enable_adaptive_mvd / enable_bawp off, enable_inter_ist / enable_inter_ddt / enable_cctx / enable_fsc / enable_idtx_intra off.

## 3. Coefficient contexts (is_inter)

- [x] 3.1 Add an `is_inter` parameter to `decode_general_intra_plane_coeffs`; select `TileTxbSkipCdf[is_inter || fsc_mode]` (index 1 for inter, fsc_mode false on this path) for plane 0/1 and pass `is_inter` to the § 5.20.7.27 nonzero pass block facts (so the luma `eobCtx = is_inter`).
- [x] 3.2 Update all six intra call sites to pass `is_inter = false` (an exact no-op: the intra fixtures decode byte-identical).

## 4. Inter residual read

- [x] 4.1 Relax the inter block skip gate to admit `skip == 0` (capture the value) in addition to `skip == 1`.
- [x] 4.2 After mode_info (read_block_tx_size reads no symbol under TX_MODE_LARGEST), read the § 5.20.7.27 residual for `skip == 0`: luma TX_64X64 then U/V TX_32X32 via the shared coefficient loop with `is_inter == true` and DCT_DCT (§ 5.20.8.3 get_tx_set returns TX_SET_DCTONLY for those sizes, so no inter_tx_type symbol).
- [x] 4.3 Reject a `skip == 0` block whose sequence enables inter-IST / inter-DDT / CCTX / FSC / IDTX-intra (those change the transform-type / coefficient read the residual decode does not model) with a structured `decode/unsupported-feature` diagnostic before any output; a `skip == 1` block reads no residual and is unaffected.

## 5. Residual reconstruction

- [x] 5.1 Add `reconstruct_inter_block_residual_into`: read the § 7.13.3.18 MC prediction block from the workspace, compose § 7.14.4 dequant + § 7.15.4 inverse transform + § 7.14.3 residual add over it, and write back (an `all_zero` plane is a no-op). The luma DCT_DCT TCQ `dqDenom` term applies only when the frame's `allow_tcq` is set; chroma never.
- [x] 5.2 Build the MC prediction workspace (unfrozen), add the residual per plane (Y/U/V), then freeze, for the `skip == 0` path; keep the `skip == 1` path on the existing freeze-after-MC.

## 6. Verification

- [x] 6.1 `splot decode --output-format raw` on `syn-2frame-inter-residual-64x64.ivf` reproduces the whole-stream md5 `ab2b067aed48cf46035fa031cefb3ab1` byte-for-byte vs avmdec == dav2d, pinned by `residual_fixture_per_frame_hash_is_stable` and the CLI test.
- [x] 6.2 The skip == 1 inter fixtures (zero-MV `4e1bd39f`, sub-pel `a0e82de3`) and the general-intra fixtures still decode byte-identical (no regression).
- [x] 6.3 `cargo xtask ci` passes; `openspec validate --all --no-interactive` passes.
