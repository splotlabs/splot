# Tasks: deblocking, GDF, CDEF frame-header params

## 1. Bookkeeping

- [x] 1.1 Confirm/assign the matrix rows (GDF/CDEF row homes);
  `openspec_change` set; register in `openspec/changes/README.md`;
  re-read § 5.18.5.2 (05 mirror :5923+), § 5.18.7.9 (:6887+),
  § 5.18.7.10 (:6983+), the call sites (:5297-5301), and the relevant
  § 5.4.10 sequence filter config verbatim.
  - **Row homes:** deblocking → `AV2-5.18.5-FILTERING`. GDF (§5.18.7.9) and
    CDEF (§5.18.7.10) have **no dedicated rows**; they are children of §5.18.7,
    so they home on `AV2-5.18.7-SEGMENTATION-TILING` (no new rows created).
    `AV2-5.18.2-FRAME-HEADER-INFO` records the advanced stop point. All three
    rows have `openspec_change = "frame-filtering-deblocking-gdf-cdef"`.
  - **README:** the validator-track changes (e.g. `mfh-frame-header-state`,
    `celu-orderhint-constraints`) are NOT listed in
    `openspec/changes/README.md`'s "Active changes" table (encoder-track only);
    tracking is via the matrix `openspec_change` field per the established
    pattern, so no README edit was needed.
  - **Gating fields (all already parsed in
    `crates/splot-core/src/headers/sequence.rs`, §5.4.10):** `enable_cdef`,
    `enable_gdf`, `gdf_unit_matches_sb_size`,
    `disable_loopfilters_across_tiles`, `cdef_on_skip_txfm` (`CdefOnSkipTxfm`),
    `df_par_bits_minus_2`; plus `enable_df_sub_pu` (§5.4.6, inter-only) and
    `single_picture_header_flag` (§5.4.1). No STOP needed — every gating field
    was present.

## 2. Parsing

- [x] 2.1 deblocking_filter_params() incl. the cur_mfh_id>0 MFH arms.
- [x] 2.2 gdf_params().
- [x] 2.3 cdef_params().
- [x] 2.4 Advance the intra-path stop status; name the next structure
  (`StoppedBeforeLoopRestorationParams`; next is `lr_params()` §5.18.7.11).

## 3. Surfacing and docs

- [x] 3.1 inspect surfaces the new fields; matrix rows advance with
  proof; generated docs regenerated; roadmap updated.

## 4. Verification

- [x] 4.1 Positive/negative/EOF tests per structure; MFH arms both ways;
  unresolvable-MFH Unknown regression.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
