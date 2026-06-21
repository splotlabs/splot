## Why

The non-intra `frame_header_info()` control region (AV2 § 5.18.2) is modeled and
reaches `InterStop::ReachedSharedTail` for the single-valid-reference implicit-map
case, but the parser then STOPS at the shared tail (`tile_info()` onward) with the
unsupported-coverage status — it never parses the § 5.18.2 shared structure cluster
or the inter-specific coding-mode arms. So no inter frame header parses end-to-end,
which blocks the next inter bricks (the §5.19 BRU tile-group arms need a COMPLETE
inter header with `NumFrameHeaderBits` known, and the §5.20 mode_info decode needs
the parsed tx_mode / reference_select / global-motion state).

The intra path already parses the full shared tail (tile_info → quantization_params
→ segmentation → setup_qm → delta_q → lossless → the loop-filter cluster → the
§5.18.2 tail) via reusable sub-parsers. The shared structure cluster is IDENTICAL
for intra and inter (§5.18.2 mirror :5183-5341); the only differences are a handful
of inter-specific arms the intra path infers to no-bit defaults.

This change wires the inter path PAST `ReachedSharedTail` through the shared tail to
a new terminal `InterHeaderComplete` for the verified minimal-tool single-reference
inter subset (the `syn-2frame-inter-64x64.ivf` fixture, bit-exact vs avmdec/dav2d),
reusing the intra shared-tail sub-parsers with the inter inputs plus the inter arms.
It is parse-only: NO decode-output change (the runtime still rejects the inter frame
at §5.20 mode_info, the next brick). The verification is a bit-level parse test on
the real fixture bytes.

## What Changes

- Add Feature ID `DECODE-INTER-HEADER-SHARED-TAIL` (an advance of
  `AV2-5.18.2-FRAME-HEADER-INFO`).
- Add `crates/splot-core/src/headers/frame/inter_shared_tail.rs`: a crate-private
  `parse_inter_shared_tail` that, after `ReachedSharedTail`, parses the § 5.18.2
  shared tail by REUSING the intra sub-parsers (`parse_tile_info` with
  `frame_is_intra == false`, `parse_quantization_params`, `parse_setup_qm_params`,
  `parse_delta_q_params`, `parse_lossless_info`, `parse_deblocking_filter_params`
  with the inter `allow_df_sub_pu` arm, `parse_gdf_params`, `parse_cdef_params`,
  `parse_lr_params`, `parse_ccso_params`, `read_tx_mode`, `parse_film_grain_config`,
  and the §5.18.9.1 `parse_global_motion_params` inter arm), plus the inter-specific
  `frame_reference_mode()` `reference_select` f(1), `skip_mode_params()`
  `skip_mode_present` f(1), and the gated `allow_bawp` / `allow_warpmv_mode` reads.
- Add the terminal `FrameHeaderParseStatus::InterHeaderComplete` and an `InterTail`
  struct + `FrameHeaderCore::inter_tail` field carrying the parsed inter-tail arms.
- Add `enable_df_sub_pu` to `CoreSeqFilterView` and `enable_bawp` /
  `enable_global_motion` to `CoreSeqInterView`, and a `read_allow_df_sub_pu`
  parameter to `parse_deblocking_filter_params` (the inter `allow_df_sub_pu` arm,
  §5.18.5.2 mirror :5935; the intra/switch/writer callers pass `false`).
- Wire `parse_inter_path` to continue into the shared tail on `ReachedSharedTail`
  via the new facts-preserving `finish_inter_control_with_tail`.

## Honest gating (the verified subset)

The shared structure cluster is reused from the INTRA-arm sub-parsers. Three of
those structures have inter-specific arms the intra parser does NOT model, which
would mis-position every following field if they fired:

- `segmentation_params()`'s enabled block: `segmentation_update_map` /
  `segmentation_temporal_update` depend on `DerivedPrimaryRefFrame` (the
  `choose_primary_secondary_ref_frame()` `RefBaseQIdx` ranking, unmodeled). The
  shared tail reads ONLY `segmentation_enabled` f(1); when it is 1, stop honestly.
- `lr_params()`'s temporal-prediction arm (gated `numRefFrames > 0`) and
  `ccso_params()`'s `reuse_ccso` / `sb_reuse_ccso` / `ccso_ref_idx` arm (gated
  `!FrameIsIntra`) become live on the inter path. An ADMISSION GATE stops honestly
  BEFORE reading any shared-tail bit when `enable_restoration && NumTotalRefs > 0`
  or `enable_ccso`, so no possibly-mis-positioned facts are ever exposed.
- `global_motion_params()`'s `use_global_motion == 1` per-reference warp models
  reach the existing honest cross-frame `GlobalMotionStop`.

Everything else (`tile_info` / `quant` / `setup_qm` / `delta_q` / lossless /
deblocking / gdf / cdef) is FrameIsIntra-arm-independent, so the intra sub-parsers
are bit-identical on the inter path. The verified `syn-2frame-inter-64x64.ivf`
fixture has restoration and CCSO disabled, so it parses to `InterHeaderComplete`;
the richer `syn-key-inter-64x64.ivf` inter frames (CCSO on) stop honestly at the
admission gate (no regression on that clean fixture).

## Impact

- Affected specs: `bitstream` (`AV2-5.18.2-FRAME-HEADER-INFO`).
- Affected code: `crates/splot-core/src/headers/frame/inter_shared_tail.rs` (new),
  `info.rs` (status + field + wiring), `filtering.rs` (deblocking inter arm +
  `CoreSeqFilterView`), `segmentation.rs` (`SegmentationParams::disabled`),
  `encoder_input.rs` (the new view fields).
- No decode-output change: the runtime still rejects the inter frame at §5.20.
