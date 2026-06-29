// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// Tunable knobs for [`annex_a_seq_payload`], a complete, frame-activatable §5.4
/// sequence header (xlayer 0, `max_tlayer_id`/`max_mlayer_id` 0, monotonic output).
#[derive(Clone, Copy)]
pub(in crate::validator::tests) struct AnnexASeq {
    pub(in crate::validator::tests) seq_id: u32,
    pub(in crate::validator::tests) profile_idc: u32,
    pub(in crate::validator::tests) level_idx: u32,
    /// `seq_tier` bit; only signaled (and thus only meaningful) when
    /// `level_idx > 3`.
    pub(in crate::validator::tests) high_tier: bool,
    pub(in crate::validator::tests) chroma_format_idc: u32,
    pub(in crate::validator::tests) bit_depth_idc: u32,
    pub(in crate::validator::tests) max_frame_width_minus_1: u32,
    pub(in crate::validator::tests) max_frame_height_minus_1: u32,
    pub(in crate::validator::tests) frame_dim_bits_minus_1: u32,
}

impl AnnexASeq {
    /// Profile 0, level 0 (2.0), Main tier, 4:2:0, 10-bit, 16x16 maximum frame.
    pub(in crate::validator::tests) fn base() -> Self {
        Self {
            seq_id: 0,
            profile_idc: 0,
            level_idx: 0,
            high_tier: false,
            chroma_format_idc: 0, // CHROMA_FORMAT_420
            bit_depth_idc: 0,     // 10-bit
            max_frame_width_minus_1: 15,
            max_frame_height_minus_1: 15,
            frame_dim_bits_minus_1: 7, // 8-bit frame dimensions
        }
    }
}

/// A complete §5.4 sequence header (non-single-picture, BLOCK_64X64, every tool
/// flag cleared) with the profile/level/tier/chroma/bit-depth and frame dimensions
/// from `o`, ready to be activated by a frame referencing `o.seq_id`. `seq_tier` is
/// read only when `seq_level_idx > 3` (§5.4.1), matching the parser.
pub(in crate::validator::tests) fn annex_a_seq_payload(o: AnnexASeq) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(o.seq_id);
    bits.f(o.profile_idc, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    bits.f(o.level_idx, 5); // seq_level_idx
    if o.level_idx > 3 {
        bits.bit(u8::from(o.high_tier)); // seq_tier
    }
    bits.uvlc(o.chroma_format_idc); // chroma_format_idc
    bits.uvlc(o.bit_depth_idc); // bit_depth_idc
    bits.f(0, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(0, 2); // max_tlayer_id
    bits.f(0, 3); // max_mlayer_id == 0
    bits.bit(1); // monotonic_output_order_flag
    bits.f(o.frame_dim_bits_minus_1, 4); // frame_width_bits_minus_1
    bits.f(o.frame_dim_bits_minus_1, 4); // frame_height_bits_minus_1
    bits.f(o.max_frame_width_minus_1, o.frame_dim_bits_minus_1 + 1);
    bits.f(o.max_frame_height_minus_1, o.frame_dim_bits_minus_1 + 1);
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    let monochrome = o.chroma_format_idc == 1; // CHROMA_FORMAT_400
    append_annex_a_child_configs(&mut bits, monochrome);
    bits.into_bytes()
}

/// A sequence-header OBU (xlayer 0) carrying [`annex_a_seq_payload`].
pub(in crate::validator::tests) fn annex_a_seq_obu(o: AnnexASeq) -> Vec<u8> {
    annex_b_obu(0x04, &annex_a_seq_payload(o))
}

/// Temporal delimiter + the [`annex_a_seq_payload`] sequence header for xlayer 0.
pub(in crate::validator::tests) fn td_and_annex_a_seq(o: AnnexASeq) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_a_seq_obu(o));
    data
}

/// [`td_and_annex_a_seq`] plus a minimal CLK frame (xlayer 0) that references
/// `o.seq_id`, *frame-confirming* the header's activation (§ 5.18.2
/// load_sequence_header) without driving the frame-core parse. The Annex A
/// *value-space* checks fire only for a frame-confirmed activation (a staged
/// OBU-order fallback is a guess that defers, § 7.3.6), so these checks need the
/// confirming frame; the static *level-limit* checks instead use the fuller
/// [`annex_a_frame_obu`], which both confirms and parses the frame core.
pub(in crate::validator::tests) fn td_seq_and_confirming_frame(o: AnnexASeq) -> Vec<u8> {
    let seq_id = o.seq_id;
    let mut data = td_and_annex_a_seq(o);
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, seq_id)); // CLK xlayer 0
    data
}

/// Appends the §5.4 child configs for a non-single-picture sequence header with
/// every tool flag cleared, gating the chroma-only reads on `monochrome` exactly as
/// the parser does, then the §5.2.1 payload tail.
pub(in crate::validator::tests) fn append_annex_a_child_configs(bits: &mut Bits, monochrome: bool) {
    bits.bit(0); // use_256x256_superblock
    bits.bit(0); // use_128x128_superblock
    if !monochrome {
        bits.bit(0); // enable_sdp
    }
    bits.bit(0); // enable_ext_partitions
    bits.bit(0); // reduce_pb_aspect_ratio
    bits.bit(0); // enable_ext_seg
    bits.bit(0); // seq_seg_info_present_flag
    bits.bit(0); // enable_dip
    bits.bit(0); // enable_intra_edge_filter
    bits.bit(0); // enable_mrls
    bits.bit(0); // enable_cfl_intra
    if !monochrome {
        bits.f(0, 2); // cfl_ds_filter_index
    }
    bits.bit(0); // enable_mhccp
    bits.bit(0); // enable_ibp
    bits.f(0, 4); // seq_enabled_motion_modes
    bits.bit(0); // enable_masked_compound
    bits.bit(0); // enable_ref_frame_mvs
    bits.f(0, 4); // order_hint_bits_minus_1
    bits.bit(0); // enable_refmvbank
    bits.bit(1); // disable_drl_reorder
    bits.bit(0); // explicit_ref_frame_map
    bits.bit(0); // explicit_num_ref_frames
    bits.f(0, 3); // long_term_frame_id_bits
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
    bits.bit(0); // enable_bru
    bits.bit(0); // enable_adaptive_mvd
    bits.bit(0); // enable_mvd_sign_derive
    bits.bit(0); // enable_flex_mvres
    bits.bit(0); // enable_global_motion
    bits.bit(0); // enable_short_refresh_frame_flags
    bits.bit(1); // seq_choose_screen_content_tools
    bits.bit(1); // seq_choose_integer_mv
    bits.bit(0); // enable_fsc
    bits.bit(0); // enable_idtx_intra
    bits.bit(0); // enable_intra_ist
    bits.bit(0); // enable_inter_ist
    if !monochrome {
        bits.bit(0); // enable_chroma_dctonly
    }
    bits.bit(0); // enable_inter_ddt
    bits.bit(0); // reduced_tx_part_set
    if !monochrome {
        bits.bit(0); // enable_cctx
    }
    bits.bit(0); // enable_tcq
    bits.bit(0); // enable_parity_hiding
    bits.bit(0); // enable_avg_cdf
    if !monochrome {
        bits.bit(0); // separate_uv_delta_q
    }
    bits.bit(1); // equal_ac_dc_q
    if !monochrome {
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
    }
    bits.bit(0); // disable_loopfilters_across_tiles
    bits.bit(0); // enable_cdef
    bits.bit(0); // enable_gdf
    bits.bit(0); // enable_restoration
    bits.bit(0); // enable_ccso
    bits.bit(0); // cdef_on_skip_txfm_always_on
    bits.bit(0); // cdef_on_skip_txfm_disabled
    bits.f(0, 2); // df_par_bits_minus_2
    bits.bit(0); // seq_tile_info_present_flag
    bits.bit(0); // film_grain_params_present
    extensible_obu_tail(bits);
}

/// A CLK frame OBU (xlayer 0) that references `seq_id`, drives `frame_size()` to an
/// override `FrameWidth` x `FrameHeight`, and reaches `tile_info()` — the parsed
/// intra-frame path the Annex A level-limit checks consume.
///
/// `frame_dim_bits` is the active sequence header's `frame_*_bits` (8 for the
/// [`AnnexASeq`] defaults). For the single-tile uniform `tile_info()` (§ 5.18.7.2),
/// `col_increment_bits` / `row_increment_bits` are the number of
/// `increment_tile_cols_log2` / `increment_tile_rows_log2` stop bits the parser
/// reads: a single `0` when the frame spans more than one superblock column (resp.
/// row) of the BLOCK_64X64 grid and the level allows a wider single tile, else `0`.
/// Use [`annex_a_single_tile_increments`] to compute these for a given level/frame.
pub(in crate::validator::tests) fn annex_a_frame_obu(
    seq_id: u32,
    width: u32,
    height: u32,
    frame_dim_bits: u32,
    col_increment_bits: u32,
    row_increment_bits: u32,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(seq_id); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame (implicit forced 0 by monotonic)
    fb.bit(1); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.f(width - 1, frame_dim_bits); // frame_width_minus_1
    fb.f(height - 1, frame_dim_bits); // frame_height_minus_1
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    fb.bit(1); // uniform_tile_spacing_flag
    for _ in 0..col_increment_bits {
        fb.bit(0); // increment_tile_cols_log2 stop bit
    }
    for _ in 0..row_increment_bits {
        fb.bit(0); // increment_tile_rows_log2 stop bit
    }
    fb.f(100, 9); // base_q_idx f(9) (10-bit sequence)
    fb.bit(0); // segmentation_enabled
    fb.bit(0); // using_qmatrix
    fb.bit(0); // delta_q_present
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// Computes the single-tile uniform `tile_info()` `increment_tile_cols_log2` /
/// `increment_tile_rows_log2` stop-bit counts for `(width, height)` at level 2.0
/// (LevelIdx 0), Main tier, BLOCK_64X64 (the [`AnnexASeq`] base). Mirrors
/// `parse_tile_layout` (§ 5.18.7.3): a single `0` stop bit when the dimension spans
/// more than one superblock and a wider single tile is allowed, else none.
pub(in crate::validator::tests) fn annex_a_single_tile_increments(
    width: u32,
    height: u32,
) -> (u32, u32) {
    fn tile_log2(blk: u32, target: u32) -> u32 {
        let mut k = 0;
        while (blk << k) < target {
            k += 1;
        }
        k
    }
    let sb_cols = (2 * ((width + 7) >> 3) + 15) >> 4;
    let sb_rows = (2 * ((height + 7) >> 3) + 15) >> 4;
    let max_tile_width_sb = (4 * 4096) >> (4 + 4); // == 64
    let max_tile_area_sb = (4u32 * 4096 * 2304) >> (2 * (4 + 2) + 2); // == 2304
    let min_log2_tile_cols = tile_log2(max_tile_width_sb, sb_cols);
    let max_log2_tile_cols = tile_log2(1, sb_cols.min(64));
    let max_log2_tile_rows = tile_log2(1, sb_rows.min(64));
    let min_log2_tiles = min_log2_tile_cols.max(tile_log2(max_tile_area_sb, sb_rows * sb_cols));
    let col_bits = u32::from(min_log2_tile_cols < max_log2_tile_cols);
    let min_log2_tile_rows = min_log2_tiles; // tile_cols_log2 == 0 for one column tile
    let row_bits = u32::from(min_log2_tile_rows < max_log2_tile_rows);
    (col_bits, row_bits)
}

#[test]
fn annex_a_flags_reserved_profile() {
    let data = td_seq_and_confirming_frame(AnnexASeq {
        profile_idc: 5,
        ..AnnexASeq::base()
    });
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/profile-reserved" && d.spec_section.as_deref() == Some("A.2")
        }),
        "report was: {report}"
    );
}

#[test]
fn annex_a_accepts_profile_4_and_30_boundary() {
    let ok = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
        profile_idc: 4,
        chroma_format_idc: 0, // 4:2:0 is allowed under profile 4
        ..AnnexASeq::base()
    }));
    assert!(
        !ok.errors().any(|d| d.rule_id == "annex-a/profile-reserved"),
        "profile 4 is defined, not reserved; report was: {ok}"
    );
    let bad = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
        profile_idc: 30,
        ..AnnexASeq::base()
    }));
    assert!(
        bad.errors()
            .any(|d| d.rule_id == "annex-a/profile-reserved"),
        "profile 30 is reserved; report was: {bad}"
    );
}

#[test]
fn annex_a_flags_chroma_format_mismatch_under_profile() {
    let data = td_seq_and_confirming_frame(AnnexASeq {
        profile_idc: 0,
        chroma_format_idc: 3, // CHROMA_FORMAT_422
        ..AnnexASeq::base()
    });
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "annex-a/profile-chroma-format-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn annex_a_profile_3_allows_422_but_not_444() {
    let ok = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
        profile_idc: 3,
        chroma_format_idc: 3, // CHROMA_FORMAT_422
        ..AnnexASeq::base()
    }));
    assert!(
        !ok.errors()
            .any(|d| d.rule_id == "annex-a/profile-chroma-format-mismatch"),
        "profile 3 allows 4:2:2; report was: {ok}"
    );
    let bad = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
        profile_idc: 3,
        chroma_format_idc: 2, // CHROMA_FORMAT_444
        ..AnnexASeq::base()
    }));
    assert!(
        bad.errors()
            .any(|d| d.rule_id == "annex-a/profile-chroma-format-mismatch"),
        "profile 3 does not allow 4:4:4; report was: {bad}"
    );
}

#[test]
fn annex_a_profile_4_allows_444_but_not_422() {
    let ok = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
        profile_idc: 4,
        chroma_format_idc: 2, // CHROMA_FORMAT_444
        ..AnnexASeq::base()
    }));
    assert!(
        !ok.errors()
            .any(|d| d.rule_id == "annex-a/profile-chroma-format-mismatch"),
        "profile 4 allows 4:4:4; report was: {ok}"
    );
    let bad = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
        profile_idc: 4,
        chroma_format_idc: 3, // CHROMA_FORMAT_422
        ..AnnexASeq::base()
    }));
    assert!(
        bad.errors()
            .any(|d| d.rule_id == "annex-a/profile-chroma-format-mismatch"),
        "profile 4 does not allow 4:2:2; report was: {bad}"
    );
}

#[test]
fn annex_a_configurable_profile_is_unconstrained() {
    let data = td_seq_and_confirming_frame(AnnexASeq {
        profile_idc: 31,
        chroma_format_idc: 3, // CHROMA_FORMAT_422
        ..AnnexASeq::base()
    });
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("annex-a/profile-")),
        "the Configurable profile is unconstrained; report was: {report}"
    );
}

#[test]
fn annex_a_flags_reserved_level() {
    let data = td_seq_and_confirming_frame(AnnexASeq {
        level_idx: 25,
        ..AnnexASeq::base()
    });
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/level-reserved" && d.spec_section.as_deref() == Some("A.4")
        }),
        "report was: {report}"
    );
}

#[test]
fn annex_a_value_space_rechecked_on_same_id_redefinition_with_different_level() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_a_seq_obu(AnnexASeq {
        seq_id: 0,
        level_idx: 0,
        ..AnnexASeq::base()
    }));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // confirm clean activation
    data.extend(annex_a_seq_obu(AnnexASeq {
        seq_id: 0,
        level_idx: 25,
        ..AnnexASeq::base()
    }));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // confirm redefinition
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/level-reserved" && d.spec_section.as_deref() == Some("A.4")
        }),
        "a same-id redefinition with a reserved seq_level_idx must re-run the Annex A \
         value-space check and flag the reserved level; report was: {report}"
    );
}

#[test]
fn annex_a_value_space_deferred_until_a_frame_confirms_a_staged_header() {
    let mut staged = temporal_delimiter_obu();
    staged.extend(annex_a_seq_obu(AnnexASeq {
        seq_id: 0,
        level_idx: 25, // reserved 22-30
        ..AnnexASeq::base()
    }));
    staged.extend(annex_a_seq_obu(AnnexASeq {
        seq_id: 1,
        level_idx: 0, // clean — defeats the sole-header decidability shortcut
        ..AnnexASeq::base()
    }));
    let staged_report = Validator::new(false).validate_bytes(&staged);
    assert!(
        !staged_report
            .errors()
            .any(|d| d.rule_id.starts_with("annex-a/")),
        "two headers staged before any frame must not fire any Annex A value-space \
         diagnostic against the OBU-order fallback guess; report was: {staged_report}"
    );

    let mut confirmed = staged.clone();
    confirmed.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    let confirmed_report = Validator::new(false).validate_bytes(&confirmed);
    assert!(
        confirmed_report.errors().any(|d| {
            d.rule_id == "annex-a/level-reserved" && d.spec_section.as_deref() == Some("A.4")
        }),
        "once a frame confirms the reserved-level header, the deferred Annex A check \
         must fire; report was: {confirmed_report}"
    );
}

#[test]
fn annex_a_value_space_fires_for_in_band_header_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(annex_a_seq_obu(AnnexASeq {
        seq_id: 0,
        level_idx: 25, // reserved 22-30
        ..AnnexASeq::base()
    }));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK ref in-band seq 0
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(5)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/level-reserved" && d.spec_section.as_deref() == Some("A.4")
        }),
        "an in-band reserved level activated by a frame must be flagged even when \
         external HLS declares an unrelated header; report was: {report}"
    );
}

#[test]
fn annex_a_value_space_silent_for_external_only_activation() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 5)); // ref external-only seq 5
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(5)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report.errors().any(|d| d.rule_id.starts_with("annex-a/")),
        "an external-only activation with no in-band header must not produce any \
         Annex A value-space diagnostic; report was: {report}"
    );
}

#[test]
fn annex_a_value_space_redefinition_rechecks_all_layers_using_the_id() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_a_seq_obu(AnnexASeq {
        seq_id: 0,
        level_idx: 0,
        ..AnnexASeq::base()
    }));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 ref seq 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 0)); // CLK xlayer 1 ref seq 0
    data.extend(temporal_delimiter_obu());
    data.extend(annex_a_seq_obu(AnnexASeq {
        seq_id: 0,
        level_idx: 25,
        ..AnnexASeq::base()
    }));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // re-activate seq 0 (xlayer 0)
    let report = Validator::new(false).validate_bytes(&data);
    let reserved_level_count = report
        .errors()
        .filter(|d| {
            d.rule_id == "annex-a/level-reserved" && d.spec_section.as_deref() == Some("A.4")
        })
        .count();
    assert!(
        reserved_level_count >= 2,
        "a redefinition flipping the level to reserved must re-run the Annex A check \
         for every extended layer (0 and 1) the id is active for, firing once per \
         affected layer key; got {reserved_level_count} reserved-level diagnostics. \
         report was: {report}"
    );
}

#[test]
fn annex_a_accepts_level_21_and_31() {
    for level_idx in [21u32, 31] {
        let report =
            Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
                level_idx,
                ..AnnexASeq::base()
            }));
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "annex-a/level-reserved"),
            "level {level_idx} is valid; report was: {report}"
        );
    }
    let bad = Validator::new(false).validate_bytes(&td_seq_and_confirming_frame(AnnexASeq {
        level_idx: 22,
        ..AnnexASeq::base()
    }));
    assert!(
        bad.errors().any(|d| d.rule_id == "annex-a/level-reserved"),
        "level 22 is reserved; report was: {bad}"
    );
}

#[test]
fn annex_a_high_tier_below_level_4_0_is_unreachable_in_syntax() {
    let report = Validator::new(false).validate_bytes(&td_and_annex_a_seq(AnnexASeq {
        level_idx: 0,
        high_tier: true, // not signaled at level 0; the parser infers Main tier
        ..AnnexASeq::base()
    }));
    assert!(
        report
            .warnings()
            .all(|d| d.rule_id != "annex-a/high-tier-below-4-0"),
        "seq_tier is not signaled below level_idx 4, so High tier below 4.0 is \
         unreachable; report was: {report}"
    );
    assert!(
        conformant_apart_from_header_only_celu(&report),
        "a level-0 Main-tier stream is conformant; report was: {report}"
    );
}

#[test]
fn annex_a_high_tier_at_level_4_0_is_accepted() {
    let report = Validator::new(false).validate_bytes(&td_and_annex_a_seq(AnnexASeq {
        level_idx: 4,
        high_tier: true,
        ..AnnexASeq::base()
    }));
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/high-tier-below-4-0"),
        "High tier at level 4.0 is allowed; report was: {report}"
    );
    assert!(
        report
            .warnings()
            .all(|d| d.rule_id != "annex-a/high-tier-below-4-0"),
        "no high-tier warning at level 4.0; report was: {report}"
    );
}
