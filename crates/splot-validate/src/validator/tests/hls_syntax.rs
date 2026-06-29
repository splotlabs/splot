// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

pub(in crate::validator::tests) fn msdo_syntax_bits(num_streams_minus_2: u32) -> Bits {
    let mut bits = Bits::default();
    bits.f(num_streams_minus_2, 3); // num_streams_minus_2
    bits.f(0, 5); // multistream_profile_idc
    bits.f(0, 5); // multistream_level_idx
    bits.bit(0); // multistream_tier
    bits.bit(1); // multistream_even_allocation_flag
    for _ in 0..(num_streams_minus_2 + 2) {
        bits.f(0, 5); // sub_xlayer_id
        bits.f(0, 5); // sub_stream_max_profile
        bits.f(0, 5); // sub_stream_max_level
        bits.bit(0); // sub_stream_max_tier
    }
    bits.bit(0); // multistream_doh_constraint_flag
    bits
}

pub(in crate::validator::tests) fn msdo_payload(num_streams_minus_2: u32) -> Vec<u8> {
    let mut bits = msdo_syntax_bits(num_streams_minus_2);
    bits.bit(1); // trailing_one_bit (valid trailing_bits)
    bits.into_bytes()
}

#[test]
fn hls_duplicate_temporal_delimiter_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-order/duplicate-temporal-delimiter"),
        "report was: {report}"
    );
}

#[test]
fn hls_repeated_identical_sequence_header_is_accepted() {
    let mut data = temporal_delimiter_obu();
    let payload = sequence_header_payload_with_id(0, 0, 0);
    data.extend(annex_b_obu(0x04, &payload));
    data.extend(annex_b_obu(0x04, &payload));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn hls_repeated_non_identical_sequence_header_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn hls_msdo_non_global_layer_id_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(20, 0, 0, 5),
        &msdo_payload(0),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "msdo/non-global-layer-id"),
        "report was: {report}"
    );
}

#[test]
fn hls_msdo_too_many_streams_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x50, &msdo_payload(3)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "msdo/too-many-streams"),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "msdo/non-global-layer-id"),
        "global MSDO must not be flagged for layer ids; report was: {report}"
    );
}

#[test]
fn hls_msdo_malformed_trailing_bits_is_flagged() {
    let mut bits = msdo_syntax_bits(0);
    bits.bit(1); // trailing_one_bit
    bits.bit(1); // trailing_zero_bit must be 0 -> violation
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x50, &bits.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "trailing-bits/zero-bit-not-zero"),
        "report was: {report}"
    );
}

#[test]
fn hls_well_formed_msdo_has_no_trailing_bits_error() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x50, &msdo_payload(0)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("trailing-bits/")),
        "report was: {report}"
    );
}

#[test]
fn hls_repeated_sequence_header_across_temporal_units_without_clk_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(annex_b_obu(0x10, &[]));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
        "a non-identical repeat in the same coded video sequence (no CLK in \
         temporal unit 2) must be flagged at the end-of-stream flush; report \
         was: {report}"
    );
}

#[test]
fn hls_cross_temporal_unit_repeat_is_flushed_at_next_temporal_delimiter() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));

    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| d.rule_id == "hls/repeated-sequence-header-not-identical")
            .count(),
        1,
        "exactly the A-vs-B repeat must be flagged (the identical params-B \
         repeat in temporal unit 3 must not be); report was: {report}"
    );
}

#[test]
fn hls_clk_for_other_xlayer_does_not_end_coded_video_sequence() {
    pub(in crate::validator::tests) fn stream(clk_xlayer: u8) -> Vec<u8> {
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1)); // id 0, params A
        data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
        data.extend(temporal_delimiter_obu());
        data.extend(sequence_header_obu_for_xlayer(0, 0, 0)); // id 0, params B
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(4, 0, 0, clk_xlayer),
            &[],
        ));
        data
    }

    let other_layer = Validator::new(false).validate_bytes(&stream(1));
    assert!(
        other_layer
            .errors()
            .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
        "a CLK for xlayer 1 must not scope away xlayer 0's repeat; report was: \
         {other_layer}"
    );

    let same_layer = Validator::new(false).validate_bytes(&stream(0));
    assert!(
        !same_layer
            .errors()
            .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
        "a CLK for xlayer 0 starts a new coded video sequence at temporal unit 2, \
         so the params-B header joins it; report was: {same_layer}"
    );
}

#[test]
fn sequence_header_truncated_child_config_is_flagged() {
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id
    bits.f(0, 5); // seq_profile_idc
    bits.bit(1); // single_picture_header_flag
    bits.f(0, 5); // seq_level_idx
    bits.uvlc(0); // chroma_format_idc
    bits.uvlc(0); // bit_depth_idc
    bits.f(3, 4); // frame_width_bits_minus_1
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(15, 4); // max_frame_width_minus_1
    bits.f(7, 4); // max_frame_height_minus_1
    bits.bit(0); // seq_cropping_window_present_flag (no child config follows)
    let data = annex_b_obu(0x04, &bits.into_bytes());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "bitstream/parse-error"),
        "report was: {report}"
    );
}

/// Builds a single-picture `sequence_header_obu()` payload (16x8, BLOCK_64X64,
/// level 0) with optional segment info and tile config, plus the §5.2.1 payload
/// tail. Mirrors the splot-core still-picture parser field-for-field.
pub(in crate::validator::tests) fn single_picture_seq_header_payload(
    seg_present: bool,
    tile_present: bool,
    uniform: bool,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id
    bits.f(0, 5); // seq_profile_idc
    bits.bit(1); // single_picture_header_flag
    bits.f(0, 5); // seq_level_idx (single picture -> no seq_tier)
    bits.uvlc(0); // chroma_format_idc = 420
    bits.uvlc(0); // bit_depth_idc
    bits.f(3, 4); // frame_width_bits_minus_1
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(15, 4); // max_frame_width_minus_1 -> 16
    bits.f(7, 4); // max_frame_height_minus_1 -> 8
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // use_256x256_superblock
    bits.bit(0); // use_128x128_superblock -> BLOCK_64X64
    bits.bit(0); // enable_sdp
    bits.bit(0); // enable_ext_partitions
    bits.bit(0); // reduce_pb_aspect_ratio
    bits.bit(0); // enable_ext_seg -> MaxSegments = 8
    bits.bit(u8::from(seg_present)); // seq_seg_info_present_flag
    if seg_present {
        bits.bit(0); // seq_allow_seg_info_change
        for _ in 0..(8 * 3) {
            bits.bit(0); // seg_info(8): all features disabled
        }
    }
    bits.bit(0); // enable_dip
    bits.bit(0); // enable_intra_edge_filter
    bits.bit(0); // enable_mrls
    bits.bit(0); // enable_cfl_intra
    bits.f(0, 2); // cfl_ds_filter_index
    bits.bit(0); // enable_mhccp
    bits.bit(0); // enable_ibp
    bits.bit(0); // enable_refmvbank
    bits.bit(1); // disable_drl_reorder -> DRL_REORDER_DISABLED
    bits.bit(0); // seq_max_bvp_drl_bits_minus_1 = ns(3) -> 0
    bits.bit(0); // allow_frame_max_bvp_drl_bits
    bits.bit(0); // enable_bawp
    bits.bit(0); // enable_fsc
    bits.bit(0); // enable_idtx_intra
    bits.bit(0); // enable_intra_ist
    bits.bit(0); // enable_inter_ist
    bits.bit(0); // enable_chroma_dctonly
    bits.bit(0); // reduced_tx_part_set
    bits.bit(0); // enable_cctx
    bits.bit(0); // enable_tcq
    bits.bit(0); // enable_parity_hiding
    bits.bit(0); // separate_uv_delta_q
    bits.bit(1); // equal_ac_dc_q
    bits.f(0, 5); // base_uv_ac_delta_q
    bits.bit(0); // uv_ac_delta_q_enabled
    bits.bit(0); // disable_loopfilters_across_tiles
    bits.bit(0); // enable_cdef
    bits.bit(0); // enable_gdf
    bits.bit(0); // enable_restoration
    bits.bit(0); // enable_ccso
    bits.f(0, 2); // df_par_bits_minus_2
    bits.bit(u8::from(tile_present)); // seq_tile_info_present_flag
    if tile_present {
        bits.bit(0); // allow_tile_info_change
        bits.bit(u8::from(uniform)); // uniform_tile_spacing_flag
    }
    bits.bit(0);
    bits.bit(0); // obu_extension_flag = 0
    bits.bit(1); // trailing_one_bit
    bits.into_bytes()
}

#[test]
fn sequence_header_with_uniform_tile_config_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &single_picture_seq_header_payload(false, true, true),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("tile-params/")),
        "report was: {report}"
    );
    assert!(
        conformant_apart_from_header_only_celu(&report),
        "report was: {report}"
    );
}

#[test]
fn sequence_header_with_nonuniform_tile_config_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &single_picture_seq_header_payload(false, true, false),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("tile-params/")),
        "report was: {report}"
    );
    assert!(
        conformant_apart_from_header_only_celu(&report),
        "report was: {report}"
    );
}

#[test]
fn sequence_header_with_segment_info_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &single_picture_seq_header_payload(true, false, false),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        conformant_apart_from_header_only_celu(&report),
        "report was: {report}"
    );
}

#[test]
fn sequence_header_malformed_tail_after_segment_info_is_flagged() {
    let mut payload = single_picture_seq_header_payload(true, false, false);
    payload.push(0xFF);
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &payload));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id.starts_with("trailing-bits/")),
        "report was: {report}"
    );
}

#[test]
fn hls_mfh_nonzero_obu_extension_flag_is_flagged() {
    let mut bits = Bits::default();
    bits.uvlc(0); // mfh_seq_header_id
    bits.uvlc(0); // mfh_id_minus_1
    bits.bit(0); // mfh_frame_size_present_flag
    bits.bit(0); // mfh_deblocking_filter_update
    bits.bit(0); // mfh_seg_info_present_flag -> fully parsed
    bits.bit(1); // obu_extension_flag = 1 -> §6.2.1 violation
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x0C, &bits.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-header/extension-flag-not-zero"),
        "report was: {report}"
    );
}

#[test]
fn hls_mfh_out_of_range_ids_are_flagged() {
    let mut bits = Bits::default();
    bits.uvlc(16); // mfh_seq_header_id (>= MAX_SEQ_NUM)
    bits.uvlc(16); // mfh_id_minus_1 -> mfhId = 17 (>= MAX_MFH_NUM)
    bits.bit(0); // mfh_frame_size_present_flag
    bits.bit(0); // mfh_deblocking_filter_update
    bits.bit(0); // mfh_seg_info_present_flag
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x0C, &bits.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "mfh/seq-header-id-out-of-range"),
        "report was: {report}"
    );
    assert!(
        report.errors().any(|d| d.rule_id == "mfh/id-out-of-range"),
        "report was: {report}"
    );
}
