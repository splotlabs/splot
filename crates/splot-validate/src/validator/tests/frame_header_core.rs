// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

pub(in crate::validator::tests) const BRIDGE_HEADER: u8 = 0x4C;
pub(in crate::validator::tests) const RAS_HEADER: u8 = 0x54;

/// Tunable knobs for [`frame_core_seq_payload`]; defaults from [`FrameCoreSeq::base`].
#[derive(Clone, Copy)]
pub(in crate::validator::tests) struct FrameCoreSeq {
    pub(in crate::validator::tests) seq_id: u32,
    pub(in crate::validator::tests) frame_width_bits_minus_1: u32,
    pub(in crate::validator::tests) frame_height_bits_minus_1: u32,
    pub(in crate::validator::tests) max_frame_width_minus_1: u32,
    pub(in crate::validator::tests) max_frame_height_minus_1: u32,
    pub(in crate::validator::tests) order_hint_bits_minus_1: u32,
    pub(in crate::validator::tests) num_ref_frames_minus_1: u32,
    pub(in crate::validator::tests) long_term_frame_id_bits: u32,
    pub(in crate::validator::tests) still_picture: bool,
    pub(in crate::validator::tests) enable_short_refresh_frame_flags: bool,
    /// `enable_ccso` (§ 5.4.10): when set, the sequence filter config signals CCSO
    /// and `ccso_unit_matches_sb_size`, so the frame's `ccso_params()` reads.
    pub(in crate::validator::tests) enable_ccso: bool,
    /// `film_grain_params_present` (§ 5.4.1, the sequence header's last flag): when
    /// set, `film_grain_config()` reads `apply_grain` f(1) on an output frame (and,
    /// when applied, `fgm_id` f(3) + `grain_seed` f(16)), so a truncated intra / SEF
    /// tail can run out inside it.
    pub(in crate::validator::tests) film_grain_params_present: bool,
    /// When set, build a header that hits the bounded `sequence_tile_config()` residual
    /// (§ 5.4.2): a reserved `seq_level_idx` (22, no defined tile bit layout) with
    /// `seq_tile_info_present_flag == 1`. The resulting [`SequenceHeader`] has every
    /// child config present but `film_grain_params_present == None` (read after the
    /// tile config), exercising the finding-C deferral.
    pub(in crate::validator::tests) bounded_tile_config: bool,
    /// `explicit_ref_frame_map` (§ 5.4.6): when set, an inter frame reads
    /// `frame_explicit_ref_frame_map` / `num_total_refs` / `ref_frame_idx[i]` from the
    /// bitstream rather than deriving the reference map (§5.18.2 mirror :4583-4625).
    pub(in crate::validator::tests) explicit_ref_frame_map: bool,
    /// `enable_bru` (§ 5.4.6): when set, an inter frame reads the BRU triple
    /// (`use_bru` / `bru_ref` / `bru_inactive`, §5.18.2 mirror :4653-4669).
    pub(in crate::validator::tests) enable_bru: bool,
    /// `max_mlayer_id` (§ 5.4.1, mirror :387): the highest embedded layer the CVS may
    /// declare. When `> 0` the header reads two extra fields — `seq_max_mlayer_cnt_minus_1`
    /// f(CeilLog2(max_mlayer_id + 1)) right after `max_mlayer_id` (mirror :389-395) and
    /// `mlayer_dependency_present_flag` f(1) after `decoder_model_info_present_flag`
    /// (mirror :507-509, here cleared so no dependency map reads) — and, crucially, it
    /// takes the §5.18.2 refresh derivation OUT of the `OBU_RAS_FRAME && max_mlayer_id == 0`
    /// arm (mirror :4493) that reads RefValid/RefLongTermId and forces the inter parser's
    /// honest early stop. A `max_mlayer_id != 0` RAS frame falls through to the explicit
    /// SWITCH `refresh_frame_flags f(NumRefFrames)` arm (mirror :4507-4509), so the parse
    /// continues into the reference region and records `ref_frame_idx`.
    pub(in crate::validator::tests) max_mlayer_id: u32,
    /// `max_tlayer_id` (§ 5.4.1, mirror :385): the highest temporal layer the CVS may
    /// declare. When `> 0` the header reads `tlayer_dependency_present_flag` f(1) after
    /// `mlayer_dependency_present_flag` (here cleared, so the § 5.4.1 lower-triangular
    /// default `TLayerDependencyMap` fill stands).
    pub(in crate::validator::tests) max_tlayer_id: u32,
}

impl FrameCoreSeq {
    /// seq 0; 8-bit frame dimensions, 16x16 maximum; OrderHintBits = 1,
    /// NumRefFrames = 8; no long-term ids; not still-picture; full refresh signaling;
    /// CCSO disabled.
    pub(in crate::validator::tests) fn base() -> Self {
        Self {
            seq_id: 0,
            frame_width_bits_minus_1: 7,
            frame_height_bits_minus_1: 7,
            max_frame_width_minus_1: 15,
            max_frame_height_minus_1: 15,
            order_hint_bits_minus_1: 0,
            num_ref_frames_minus_1: 7,
            long_term_frame_id_bits: 0,
            still_picture: false,
            enable_short_refresh_frame_flags: false,
            enable_ccso: false,
            film_grain_params_present: false,
            bounded_tile_config: false,
            explicit_ref_frame_map: false,
            enable_bru: false,
            max_mlayer_id: 0,
            max_tlayer_id: 0,
        }
    }
}

/// A fully-parseable §5.4 sequence header (xlayer 0, max_tlayer/mlayer 0,
/// monotonic output) with a tunable inter config and frame dimensions, for
/// exercising the frame-header core diagnostics.
pub(in crate::validator::tests) fn frame_core_seq_payload(o: FrameCoreSeq) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(o.seq_id);
    bits.f(0, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    if o.bounded_tile_config {
        bits.f(22, 5); // seq_level_idx (reserved)
        bits.bit(0); // seq_tier (signaled because seq_level_idx > 3, not single-picture)
    } else {
        bits.f(0, 5); // seq_level_idx
    }
    bits.uvlc(0); // chroma_format_idc
    bits.uvlc(0); // bit_depth_idc
    bits.f(0, 3); // seq_lcr_id
    bits.bit(u8::from(o.still_picture)); // still_picture
    bits.f(o.max_tlayer_id, 2); // max_tlayer_id (mirror :385)
    bits.f(o.max_mlayer_id, 3); // max_mlayer_id (mirror :387)
    if o.max_mlayer_id > 0 {
        bits.f(o.max_mlayer_id, ceil_log2_u32(o.max_mlayer_id + 1));
    }
    bits.bit(1); // monotonic_output_order_flag
    bits.f(o.frame_width_bits_minus_1, 4);
    bits.f(o.frame_height_bits_minus_1, 4);
    bits.f(o.max_frame_width_minus_1, o.frame_width_bits_minus_1 + 1);
    bits.f(o.max_frame_height_minus_1, o.frame_height_bits_minus_1 + 1);
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    if o.max_mlayer_id > 0 {
        bits.bit(0); // mlayer_dependency_present_flag
    }
    if o.max_tlayer_id > 0 {
        bits.bit(0); // tlayer_dependency_present_flag
    }
    bits.bit(0); // use_256x256_superblock
    bits.bit(0); // use_128x128_superblock
    bits.bit(0); // enable_sdp
    bits.bit(0); // enable_ext_partitions
    bits.bit(0); // reduce_pb_aspect_ratio
    bits.bit(0); // enable_ext_seg
    bits.bit(0); // seq_seg_info_present_flag
    bits.bit(0); // enable_dip
    bits.bit(0); // enable_intra_edge_filter
    bits.bit(0); // enable_mrls
    bits.bit(0); // enable_cfl_intra
    bits.f(0, 2); // cfl_ds_filter_index
    bits.bit(0); // enable_mhccp
    bits.bit(0); // enable_ibp
    bits.f(0, 4); // seq_enabled_motion_modes
    bits.bit(0); // enable_masked_compound
    bits.bit(0); // enable_ref_frame_mvs
    bits.f(o.order_hint_bits_minus_1, 4); // order_hint_bits_minus_1
    bits.bit(0); // enable_refmvbank
    bits.bit(1); // disable_drl_reorder
    bits.bit(u8::from(o.explicit_ref_frame_map)); // explicit_ref_frame_map
    bits.bit(1); // explicit_num_ref_frames
    bits.f(o.num_ref_frames_minus_1, 4); // num_ref_frames_minus_1
    bits.f(o.long_term_frame_id_bits, 3); // long_term_frame_id_bits
    bits.f(0, 2); // seq_max_drl_bits_minus_1 (ns(5) -> 0)
    bits.bit(0); // allow_frame_max_drl_bits
    bits.bit(0); // seq_max_bvp_drl_bits_minus_1 (ns(3) -> 0)
    bits.bit(0); // allow_frame_max_bvp_drl_bits
    bits.f(0, 2); // num_same_ref_compound
    bits.bit(0); // enable_tip
    bits.bit(0); // enable_mv_traj
    bits.bit(0); // enable_bawp
    bits.bit(0); // enable_cwp
    bits.bit(0); // enable_imp_msk_bld
    bits.bit(0); // enable_df_sub_pu
    bits.f(0, 2); // enable_opfl_refine
    bits.bit(0); // enable_refinemv
    bits.bit(u8::from(o.enable_bru)); // enable_bru
    bits.bit(0); // enable_adaptive_mvd
    bits.bit(0); // enable_mvd_sign_derive
    bits.bit(0); // enable_flex_mvres
    bits.bit(0); // enable_global_motion
    bits.bit(u8::from(o.enable_short_refresh_frame_flags)); // enable_short_refresh_frame_flags
    bits.bit(1); // seq_choose_screen_content_tools
    bits.bit(1); // seq_choose_integer_mv
    bits.bit(0); // enable_fsc
    bits.bit(0); // enable_idtx_intra
    bits.bit(0); // enable_intra_ist
    bits.bit(0); // enable_inter_ist
    bits.bit(0); // enable_chroma_dctonly
    bits.bit(0); // enable_inter_ddt
    bits.bit(0); // reduced_tx_part_set
    bits.bit(0); // enable_cctx
    bits.bit(0); // enable_tcq
    bits.bit(0); // enable_parity_hiding
    bits.bit(0); // enable_avg_cdf
    bits.bit(0); // separate_uv_delta_q
    bits.bit(1); // equal_ac_dc_q
    bits.f(0, 5); // base_uv_ac_delta_q
    bits.bit(0); // uv_ac_delta_q_enabled
    bits.bit(0); // disable_loopfilters_across_tiles
    bits.bit(0); // enable_cdef
    bits.bit(0); // enable_gdf
    bits.bit(0); // enable_restoration
    bits.bit(u8::from(o.enable_ccso)); // enable_ccso
    if o.enable_ccso {
        bits.bit(0); // ccso_unit_matches_sb_size (no effect on the offset-loop reads)
    }
    bits.bit(0); // cdef_on_skip_txfm_always_on
    bits.bit(0); // cdef_on_skip_txfm_disabled
    bits.f(0, 2); // df_par_bits_minus_2
    if o.bounded_tile_config {
        bits.bit(1); // seq_tile_info_present_flag
        bits.bit(0); // allow_tile_info_change
        extensible_obu_tail(&mut bits);
        return bits.into_bytes();
    }
    bits.bit(0); // seq_tile_info_present_flag
    bits.bit(u8::from(o.film_grain_params_present)); // film_grain_params_present
    extensible_obu_tail(&mut bits);
    bits.into_bytes()
}

/// A temporal delimiter followed by a `frame_core_seq_payload` sequence header.
pub(in crate::validator::tests) fn td_and_frame_core_seq(o: FrameCoreSeq) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &frame_core_seq_payload(o)));
    data
}

/// Appends the § 5.18.2 intra structure cluster the core parser consumes after
/// `disable_cdf_update` for a [`frame_core_seq_payload`] sequence (10-bit,
/// 4:2:0, BLOCK_64X64, no sequence tile/segmentation info, every optional
/// quantizer read disabled, `enable_cdef == enable_gdf == 0`): a single-tile
/// `tile_info()` (§ 5.18.7.2; `uniform_tile_spacing_flag` plus `col_increment_bits`
/// zero increment bits — one for the 256-wide frame, none for the 16x16 default),
/// `base_q_idx` f(9) (§ 5.18.6.1), `segmentation_enabled = 0` (§ 5.18.7.1),
/// `using_qmatrix = 0` (§ 5.18.6.2), `delta_q_present = 0` (§ 5.18.7.8), and the
/// loop-filter cluster: `deblocking_filter_params()` reads `apply_deblocking_filter`
/// `[0]`/`[1]` (both 0 — nonzero `base_q_idx` keeps `CodedLossless == 0`), while
/// `gdf_params()` / `cdef_params()` read nothing (GDF / CDEF disabled). With a
/// nonzero `base_q_idx` the § 5.18.2 lossless tail reads no further bits.
pub(in crate::validator::tests) fn intra_structure_tail(fb: &mut Bits, col_increment_bits: u32) {
    fb.bit(1); // uniform_tile_spacing_flag (tile_info)
    for _ in 0..col_increment_bits {
        fb.bit(0); // increment_tile_cols_log2 = 0
    }
    fb.f(100, 9); // base_q_idx f(9) (10-bit sequence)
    fb.bit(0); // segmentation_enabled
    fb.bit(0); // using_qmatrix
    fb.bit(0); // delta_q_present
    fb.bit(0); // apply_deblocking_filter[0]
    fb.bit(0); // apply_deblocking_filter[1]
}

#[test]
fn validator_flags_ras_requires_long_term_frame_id_bits() {
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(RAS_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/ras-requires-long-term-frame-id-bits"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_bridge_ref_index_out_of_range() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6 (non-power-of-2)
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    fb.f(6, 3); // bridge_frame_ref_idx == 6 (>= NumRefFrames 6)
    fb.bit(0); // bridge_frame_overwrite_flag == 0 -> refresh = 1 << 6 (no bits)
    fb.f(0, 8); // bridge_frame_width_minus_1 f(frame_width_bits == 8)
    fb.f(0, 8); // bridge_frame_height_minus_1 f(frame_height_bits == 8)
    data.extend(annex_b_obu(BRIDGE_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/bridge-ref-index-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_frame_size_exceeds_sequence_max() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame (implicit forced 0 by monotonic)
    fb.bit(1); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.f(256 - 1, 8); // frame_width_minus_1 -> FrameWidth 256 (> max 16)
    fb.f(8 - 1, 8); // frame_height_minus_1 -> FrameHeight 8
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 1);
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_frame_size_within_sequence_max() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame
    fb.bit(1); // frame_size_override_flag
    fb.f(0, 1); // order_hint
    fb.f(16 - 1, 8); // frame_width_minus_1 -> FrameWidth 16 (== max)
    fb.f(16 - 1, 8); // frame_height_minus_1 -> FrameHeight 16 (== max)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
}

#[test]
fn validator_frame_size_check_fires_against_bounded_tile_config_sequence() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        bounded_tile_config: true,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame (implicit forced 0 by monotonic)
    fb.bit(1); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.f(256 - 1, 8); // frame_width_minus_1 -> FrameWidth 256 (> max 16)
    fb.f(8 - 1, 8); // frame_height_minus_1 -> FrameHeight 8
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 1); // 256-wide -> one tile column increment bit
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "a frame against a bounded-stop sequence header must still fire the frame-size \
         check (the grain flag does not gate the control region); report was: {report}"
    );
}

/// Builds a CLK frame whose §5.18.2 intra tail parses cleanly through cdef and
/// lr_params() (restoration disabled), then `ccso_params()` for a CCSO-enabled
/// sequence: `ccso_frame_flag = 1`, plane 0 enabled in the `!ccso_bo_only` arm with the
/// caller's `ccso_ext_filter` and `ccso_max_band_log2`, planes 1/2 disabled. Uses
/// `ccso_scale_idx = 0`, `ccso_quant_idx = 0` so `quantStep = CCSO_Quant_Sz[0][0] == 16`
/// (nonzero → `ccso_edge_clf` read) and `ccso_edge_clf = 0` so `maxEdgeInterval = 3`. The
/// offset loop then reads `3 * 3 * (1 << ccso_max_band_log2)` `ccso_offset_idx` tu(7)
/// values (all 0 -> a single `0` bit each).
pub(in crate::validator::tests) fn frame_with_ccso_plane0(
    ccso_ext_filter: u32,
    ccso_max_band_log2: u32,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    fb.bit(1); // ccso_frame_flag
    fb.bit(1); // ccso_planes[0]
    fb.bit(0); // ccso_bo_only[0] == 0
    fb.f(0, 2); // ccso_scale_idx[0] == 0
    fb.f(0, 2); // ccso_quant_idx[0] == 0 -> CCSO_Quant_Sz[0][0] == 16 != 0
    fb.f(ccso_ext_filter, 3); // ccso_ext_filter[0]
    fb.bit(0); // ccso_edge_clf[0] == 0 (quantStep != 0) -> maxEdgeInterval = 3
    fb.f(ccso_max_band_log2, 2); // ccso_max_band_log2[0] (n = 2, !ccso_bo_only)
    let max_band = 1u32 << ccso_max_band_log2;
    for _ in 0..(3 * 3 * max_band) {
        fb.bit(0); // ccso_offset_idx tu(7) == 0
    }
    fb.bit(0); // ccso_planes[1]
    fb.bit(0); // ccso_planes[2]
    fb.f(0, 8);
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

#[test]
fn validator_flags_ccso_ext_filter_reserved() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        enable_ccso: true,
        ..FrameCoreSeq::base()
    });
    data.extend(frame_with_ccso_plane0(7, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/ccso-ext-filter-reserved"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_ccso_ext_filter_within_range() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        enable_ccso: true,
        ..FrameCoreSeq::base()
    });
    data.extend(frame_with_ccso_plane0(6, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/ccso-ext-filter-reserved"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_ccso_max_band_out_of_range() {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    fb.bit(1); // ccso_frame_flag
    fb.bit(1); // ccso_planes[0]
    fb.bit(1); // ccso_bo_only[0] == 1 -> quant/ext/edge_clf inferred 0, maxEdgeInterval 1
    fb.f(0, 2); // ccso_scale_idx[0]
    fb.f(7, 3); // ccso_max_band_log2[0] f(3) == 7 -> 1 << 7 == 128 > CCSO_BAND_NUM
    for _ in 0..128 {
        fb.bit(0);
    }
    fb.bit(0); // ccso_planes[1]
    fb.bit(0); // ccso_planes[2]
    fb.f(0, 8); // padding
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        enable_ccso: true,
        ..FrameCoreSeq::base()
    });
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/ccso-max-band-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_ccso_max_band_within_range() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        enable_ccso: true,
        ..FrameCoreSeq::base()
    });
    data.extend(frame_with_ccso_plane0(0, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/ccso-max-band-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_frame_size_exceeds_max_when_truncated_inside_deblocking() {
    let seq = FrameCoreSeq {
        order_hint_bits_minus_1: 1,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame (implicit forced 0 by monotonic)
    fb.bit(1); // frame_size_override_flag
    fb.f(0, 2); // order_hint f(OrderHintBits == 2)
    fb.f(256 - 1, 8); // frame_width_minus_1 -> FrameWidth 256 (> max 16)
    fb.f(8 - 1, 8); // frame_height_minus_1 -> FrameHeight 8
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    fb.bit(1); // uniform_tile_spacing_flag (tile_info)
    fb.bit(0); // increment_tile_cols_log2 = 0 (256-wide -> 1 increment bit)
    fb.f(100, 9); // base_q_idx f(9)
    fb.bit(0); // segmentation_enabled
    fb.bit(0); // using_qmatrix
    fb.bit(0); // delta_q_present -> ends on bit 40 (byte boundary)
    assert_eq!(fb.bit_len(), 40, "the body must end on a byte boundary");
    let payload = fb.into_bytes();
    data.extend(annex_b_obu(CLK_HEADER, &payload));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "a frame-size violation truncated inside deblocking must still fire; report was: {report}"
    );
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/truncated-frame-header"),
        "a payload truncated inside the loop-filter cluster must fire \
         truncated-frame-header; report was: {report}"
    );
}

#[test]
fn validator_flags_truncated_frame_header_inside_intra_tail() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // immediate_output_frame == 1 (output frame -> apply_grain readable)
    fb.bit(0); // frame_size_override_flag == 0 (cur_mfh_id == 0 -> max dims 16x16)
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    fb.bit(0); // tx_mode_select = 0
    fb.f(0, 2); // reduced_tx_set = 0
    fb.bit(1); // apply_grain = 1
    fb.f(0, 3); // fgm_id = 0
    fb.f(0, 8); // only 8 of 16 grain_seed bits, then truncation
    let total_bits = fb.bit_len();
    let mut payload = fb.into_bytes();
    payload.truncate(total_bits / 8); // drop the partial trailing byte -> grain_seed overruns
    data.extend(annex_b_obu(CLK_HEADER, &payload));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/truncated-frame-header"),
        "a payload truncated inside the §5.18.2 intra tail must fire \
         truncated-frame-header; report was: {report}"
    );
}

#[test]
fn validator_flags_truncated_frame_header_inside_sef_film_grain() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    let mut fb = Bits::default();
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.f(0, 3); // frame_to_show_map_idx f(CeilLog2(NumRefFrames == 8) == 3)
    fb.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
    fb.bit(1); // apply_grain = 1
    fb.f(0, 3); // fgm_id = 0
    fb.f(0, 8); // only 8 of 16 grain_seed bits, then truncation
    let total_bits = fb.bit_len();
    let mut payload = fb.into_bytes();
    payload.truncate(total_bits / 8); // drop the partial trailing byte -> grain_seed overruns
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &payload));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/truncated-frame-header"),
        "a SEF payload truncated inside film_grain_config() must fire \
         truncated-frame-header; report was: {report}"
    );
}

/// A complete conformant REGULAR_SEF whose film_grain_config() applies grain at
/// `fgm_id`: cur_mfh_id / seq ref, frame_to_show_map_idx, derive_sef_order_hint == 1,
/// then apply_grain f(1) == 1, fgm_id f(3), grain_seed f(16), and a §5.2.3
/// trailing_one_bit.
pub(in crate::validator::tests) fn sef_with_applied_grain(fgm_id: u8) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.f(0, 3); // frame_to_show_map_idx f(3)
    fb.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
    fb.bit(1); // apply_grain == 1
    fb.f(u32::from(fgm_id), 3); // fgm_id f(3)
    fb.f(0xABCD, 16); // grain_seed f(16)
    fb.bit(1); // §5.2.3 trailing_one_bit
    annex_b_obu(REGULAR_SEF_HEADER, &fb.into_bytes())
}

#[test]
fn validator_flags_film_grain_model_unavailable() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(sef_with_applied_grain(5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "frame-header/film-grain-model-unavailable"),
        "apply_grain referencing an unreceived fgm_id slot must fire \
         film-grain-model-unavailable; report was: {report}"
    );
}

#[test]
fn validator_film_grain_model_available_in_band_is_silent() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(film_grain_obu_bytes(1 << 5, 0)); // sets FilmGrainPresent[5]
    data.extend(sef_with_applied_grain(5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "frame-header/film-grain-model-unavailable"),
        "a received in-band film grain model must NOT fire the unavailable check; \
         report was: {report}"
    );
}

#[test]
fn validator_film_grain_model_unavailable_fires_when_fgm_obu_follows_frame() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(sef_with_applied_grain(5));
    data.extend(film_grain_obu_bytes(1 << 5, 0)); // too late
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "frame-header/film-grain-model-unavailable"),
        "a film grain OBU after the referencing frame is not available in time; \
         report was: {report}"
    );
}

#[test]
fn validator_film_grain_model_unavailable_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet};
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(sef_with_applied_grain(5));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "frame-header/film-grain-model-unavailable"),
        "a Provided external-HLS mode must suppress the film-grain availability check \
         (film grain is inexpressible by ExternalHlsSet); report was: {report}"
    );
}

#[test]
fn validator_flags_sef_nonzero_bits_after_fields_as_trailing_bits_defect() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.f(0, 3); // frame_to_show_map_idx f(CeilLog2(NumRefFrames == 8) == 3)
    fb.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
    fb.bit(0); // would-be trailing_one_bit, but 0
    fb.f(0b101, 3); // arbitrary set bits after the SEF fields
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/sef-trailing-bits-invalid"),
        "a SEF with non-conformant trailing bits must fire \
         frame-header/sef-trailing-bits-invalid; report was: {report}"
    );
}

#[test]
fn validator_flags_sef_grain_seed_short_one_bit_as_trailing_bits_defect() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    let mut fb = Bits::default();
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.f(0, 3); // frame_to_show_map_idx
    fb.bit(1); // derive_sef_order_hint == 1
    fb.bit(1); // apply_grain = 1 (grain present + immediate_output inferred 1 for SEF)
    fb.f(0, 3); // fgm_id = 0
    fb.f(0, 15); // 15 grain_seed bits
    fb.bit(1); // the marker bit, consumed as the 16th grain_seed bit
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/sef-trailing-bits-invalid"),
        "a SEF whose grain_seed ate the trailing_one_bit must fire \
         frame-header/sef-trailing-bits-invalid; report was: {report}"
    );
}

#[test]
fn validator_sef_trailing_bits_silent_on_conformant_sef() {
    let mut grain_free = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut gf = Bits::default();
    gf.uvlc(0); // cur_mfh_id == 0
    gf.uvlc(0); // seq_header_id_in_frame_header
    gf.f(0, 3); // frame_to_show_map_idx
    gf.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
    gf.bit(1); // trailing_one_bit
    grain_free.extend(annex_b_obu(REGULAR_SEF_HEADER, &gf.into_bytes()));
    let report = Validator::new(false).validate_bytes(&grain_free);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/sef-trailing-bits-invalid"),
        "a conformant grain-free SEF must NOT fire sef-trailing-bits-invalid; \
         report was: {report}"
    );

    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut with_grain = td_and_frame_core_seq(seq);
    let mut wg = Bits::default();
    wg.uvlc(0); // cur_mfh_id == 0
    wg.uvlc(0); // seq_header_id_in_frame_header
    wg.f(0, 3); // frame_to_show_map_idx
    wg.bit(1); // derive_sef_order_hint == 1
    wg.bit(1); // apply_grain = 1
    wg.f(0, 3); // fgm_id = 0
    wg.f(0xABCD, 16); // grain_seed (full 16 bits)
    wg.bit(1); // §5.2.3 trailing_one_bit
    with_grain.extend(annex_b_obu(REGULAR_SEF_HEADER, &wg.into_bytes()));
    let report = Validator::new(false).validate_bytes(&with_grain);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/sef-trailing-bits-invalid"),
        "a conformant SEF with grain must NOT fire sef-trailing-bits-invalid; \
         report was: {report}"
    );
}

#[test]
fn validator_truncated_frame_header_silent_on_complete_and_coverage_stops() {
    let mut complete = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag == 0 (max dims 16x16)
    fb.f(0, 1); // order_hint f(1)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0); // structure + loop-filter cluster (no bits past)
    fb.bit(0); // tx_mode_select = 0
    fb.f(0, 2); // reduced_tx_set = 0
    complete.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&complete);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/truncated-frame-header"),
        "a complete intra frame header must NOT fire truncated-frame-header; report was: {report}"
    );

    let mut inter = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut ib = Bits::default();
    ib.bit(1); // is_first_tile_group
    ib.uvlc(0); // cur_mfh_id == 0
    ib.uvlc(0); // seq_header_id_in_frame_header
    ib.bit(1); // frame_is_inter == 1 -> INTER_FRAME
    ib.bit(0); // immediate_output_frame (monotonic_output -> implicit forced 0, no bit)
    ib.bit(0); // frame_size_override_flag
    ib.f(0, 1); // order_hint f(OrderHintBits == 1)
    ib.bit(0); // signal_primary_ref_frame
    ib.bit(0); // disable_cross_frame_cdf_init (not TIP)
    ib.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8) -> ends on a byte boundary
    inter.extend(annex_b_obu(RTG_HEADER, &ib.into_bytes()));
    let report = Validator::new(false).validate_bytes(&inter);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/truncated-frame-header"),
        "an inter-frame coverage stop must NOT fire truncated-frame-header; report was: {report}"
    );

    let mut mfh = td_and_frame_core_seq(FrameCoreSeq::base());
    mfh.extend(frame_obu_mfh_ref(CLK_HEADER, 1)); // cur_mfh_id == 1, no MFH OBU
    let report = Validator::new(false).validate_bytes(&mfh);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/truncated-frame-header"),
        "an unresolvable-MFH coverage stop must NOT fire truncated-frame-header; report was: {report}"
    );
}
