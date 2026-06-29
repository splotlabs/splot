// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// Appends the §5.2.1 extensible-OBU payload tail (`obu_extension_flag = 0` +
/// `trailing_one_bit`); `into_bytes` zero-pads the remainder.
pub(in crate::validator::tests) fn extensible_obu_tail(bits: &mut Bits) {
    bits.bit(0); // obu_extension_flag = 0
    bits.bit(1); // trailing_one_bit
}

/// A minimal global LCR OBU (`obu_xlayer_id == GLOBAL_XLAYER_ID == 31`).
pub(in crate::validator::tests) fn global_lcr_obu(
    global_id: u32,
    xlayer_map: u32,
    atlas_id: Option<u32>,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(global_id, 3); // lcr_global_config_record_id
    bits.f(xlayer_map, 31); // lcr_xlayer_map
    bits.bit(0); // lcr_aggregate_info_present_flag
    bits.bit(0); // lcr_seq_profile_tier_level_info_present_flag
    bits.bit(0); // lcr_global_payload_present_flag
    bits.bit(0); // lcr_dependent_xlayers_flag
    bits.bit(u8::from(atlas_id.is_some())); // lcr_global_atlas_id_present_flag
    bits.f(0, 7); // lcr_global_purpose_id
    bits.bit(0); // lcr_doh_constraint_flag
    bits.bit(0); // lcr_enforce_tile_alignment_flag
    bits.f(atlas_id.unwrap_or(0), 3); // lcr_global_atlas_id or reserved_zero_3bits
    bits.f(0, 5); // lcr_global_reserved_zero_5bits
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
}

/// A global LCR OBU whose `lcr_global_reserved_zero_5bits` is non-zero.
pub(in crate::validator::tests) fn global_lcr_obu_with_nonzero_reserved() -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(1, 3); // lcr_global_config_record_id
    bits.f(0b1, 31); // lcr_xlayer_map
    bits.bit(0); // aggregate
    bits.bit(0); // ptl
    bits.bit(0); // payload
    bits.bit(0); // dependent
    bits.bit(0); // atlas present
    bits.f(0, 7); // purpose
    bits.bit(0); // doh
    bits.bit(0); // tile alignment
    bits.f(0, 3); // reserved_zero_3bits
    bits.f(0b1_0001, 5); // lcr_global_reserved_zero_5bits != 0
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
}

/// A minimal local LCR OBU at `xlayer`.
pub(in crate::validator::tests) fn local_lcr_obu(
    xlayer: u8,
    global_id: u32,
    local_id: u32,
    local_atlas_id: Option<u32>,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(global_id, 3); // lcr_global_id
    bits.f(local_id, 3); // lcr_local_id
    bits.bit(0); // lcr_profile_tier_level_info_present_flag
    bits.bit(u8::from(local_atlas_id.is_some())); // lcr_local_atlas_id_present_flag
    bits.f(local_atlas_id.unwrap_or(0), 3); // lcr_local_atlas_id or reserved_zero_3bits
    bits.f(0, 5); // lcr_local_reserved_zero_5bits
    bits.bit(0); // lcr_rep_info_present_flag
    bits.bit(0); // lcr_xlayer_purpose_present_flag
    bits.bit(0); // lcr_xlayer_color_info_present_flag
    bits.bit(0); // lcr_embedded_layer_info_present_flag
    bits.align(); // byte_alignment()
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, xlayer), &bits.into_bytes())
}

/// A minimal SINGLE-mode atlas segment OBU at `xlayer`.
pub(in crate::validator::tests) fn atlas_obu(xlayer: u8, atlas_segment_id: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(atlas_segment_id, 3); // atlas_segment_id
    bits.uvlc(2); // ats_atlas_segment_mode_idc = SINGLE_ATLAS
    bits.uvlc(0); // ats_nominal_width_minus_1
    bits.uvlc(0); // ats_nominal_height_minus_1
    bits.bit(0); // ats_signaled_atlas_segment_ids_flag
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(17, 0, 0, xlayer), &bits.into_bytes())
}

/// An atlas segment OBU whose `ats_atlas_segment_mode_idc` is out of range (5).
pub(in crate::validator::tests) fn atlas_obu_bad_mode(xlayer: u8) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 3); // atlas_segment_id
    bits.uvlc(5); // ats_atlas_segment_mode_idc = 5 -> out of range
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(17, 0, 0, xlayer), &bits.into_bytes())
}

/// A MULTISTREAM_ATLAS OBU with a single segment, placed at `xlayer`.
pub(in crate::validator::tests) fn atlas_multistream_obu(xlayer: u8) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 3); // atlas_segment_id
    bits.uvlc(3); // ats_atlas_segment_mode_idc = MULTISTREAM_ATLAS
    bits.uvlc(0); // ats_msi_width
    bits.uvlc(0); // ats_msi_height
    bits.uvlc(0); // ats_msi_num_atlas_segments_minus_1 = 0 -> 1 segment
    bits.bit(0); // ats_msi_background_info_present_flag
    bits.f(0, 5); // ats_msi_input_stream_id
    bits.uvlc(0); // pos_x
    bits.uvlc(0); // pos_y
    bits.uvlc(0); // width
    bits.uvlc(0); // height
    bits.bit(0); // ats_signaled_atlas_segment_ids_flag
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(17, 0, 0, xlayer), &bits.into_bytes())
}

/// A BASIC_ATLAS OBU at `xlayer` whose two segments share an `ats_input_stream_id`.
pub(in crate::validator::tests) fn atlas_basic_duplicate_stream_obu(xlayer: u8) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 3); // atlas_segment_id
    bits.uvlc(1); // ats_atlas_segment_mode_idc = BASIC_ATLAS
    bits.bit(1); // ats_stream_id_present
    bits.uvlc(0); // ats_width
    bits.uvlc(0); // ats_height
    bits.uvlc(1); // ats_num_atlas_segments_minus_1 = 1 -> 2 segments
    for _ in 0..2 {
        bits.f(5, 5); // ats_input_stream_id = 5 (duplicated)
        bits.uvlc(0); // pos_x
        bits.uvlc(0); // pos_y
        bits.uvlc(0); // width
        bits.uvlc(0); // height
    }
    bits.bit(0); // ats_signaled_atlas_segment_ids_flag
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(17, 0, 0, xlayer), &bits.into_bytes())
}

/// A global MULTISTREAM_ATLAS OBU (xlayer 31) whose two segments share an
/// `ats_msi_input_stream_id`.
pub(in crate::validator::tests) fn atlas_multistream_duplicate_stream_obu() -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 3); // atlas_segment_id
    bits.uvlc(3); // ats_atlas_segment_mode_idc = MULTISTREAM_ATLAS
    bits.uvlc(0); // ats_msi_width
    bits.uvlc(0); // ats_msi_height
    bits.uvlc(1); // ats_msi_num_atlas_segments_minus_1 = 1 -> 2 segments
    bits.bit(0); // ats_msi_background_info_present_flag
    for _ in 0..2 {
        bits.f(5, 5); // ats_msi_input_stream_id = 5 (duplicated)
        bits.uvlc(0); // pos_x
        bits.uvlc(0); // pos_y
        bits.uvlc(0); // width
        bits.uvlc(0); // height
    }
    bits.bit(0); // ats_signaled_atlas_segment_ids_flag
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(17, 0, 0, 31), &bits.into_bytes())
}

/// A global LCR OBU whose `lcr_dependent_xlayers_flag` is set (no payload).
pub(in crate::validator::tests) fn global_lcr_obu_with_dependent_flag() -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(1, 3); // lcr_global_config_record_id
    bits.f(0b1, 31); // lcr_xlayer_map
    bits.bit(0); // aggregate
    bits.bit(0); // ptl
    bits.bit(0); // payload
    bits.bit(1); // lcr_dependent_xlayers_flag
    bits.bit(0); // atlas present
    bits.f(0, 7); // purpose
    bits.bit(0); // doh
    bits.bit(0); // tile alignment
    bits.f(0, 3); // reserved_zero_3bits
    bits.f(0, 5); // reserved_zero_5bits
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
}

/// A base-layer sequence header OBU at `xlayer` with `seq_lcr_id`.
pub(in crate::validator::tests) fn sequence_header_obu_with_lcr(
    xlayer: u8,
    seq_lcr_id: u32,
) -> Vec<u8> {
    let payload = sequence_header_payload_with_lcr(0, seq_lcr_id, 0, 0);
    annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
}

#[test]
fn hls_seq_lcr_missing_record_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_with_lcr(3, 5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-layer-configuration-record"),
        "report was: {report}"
    );
}

#[test]
fn lcr_seq_header_resolves_to_local_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu(3, 0, 5, None));
    data.extend(sequence_header_obu_with_lcr(3, 5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-layer-configuration-record"),
        "report was: {report}"
    );
}

#[test]
fn lcr_seq_header_resolves_to_global_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1000, None));
    data.extend(sequence_header_obu_with_lcr(3, 5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| {
            d.rule_id == "hls/unavailable-layer-configuration-record"
                || d.rule_id == "lcr/global-xlayer-map-missing-xlayer"
        }),
        "report was: {report}"
    );
}

#[test]
fn lcr_global_xlayer_map_missing_xlayer_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1, None));
    data.extend(sequence_header_obu_with_lcr(3, 5));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "lcr/global-xlayer-map-missing-xlayer"),
        "report was: {report}"
    );
}

#[test]
fn lcr_local_missing_global_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu(3, 2, 1, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "lcr/global-lcr-unavailable"),
        "report was: {report}"
    );
}

#[test]
fn lcr_local_missing_global_is_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu(3, 2, 1, None));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/global-lcr-unavailable"),
        "external HLS may supply the global LCR; report was: {report}"
    );
}

#[test]
fn atlas_local_atlas_unavailable_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu(3, 0, 1, Some(4)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "atlas/local-atlas-unavailable"),
        "report was: {report}"
    );
}

#[test]
fn atlas_local_atlas_unavailable_is_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu(3, 0, 1, Some(4)));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "atlas/local-atlas-unavailable"),
        "external HLS may supply the local atlas; report was: {report}"
    );
}

#[test]
fn atlas_local_atlas_available_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(atlas_obu(3, 4));
    data.extend(local_lcr_obu(3, 0, 1, Some(4)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "atlas/local-atlas-unavailable"),
        "report was: {report}"
    );
}

#[test]
fn lcr_global_xlayer_map_missing_xlayer_is_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(5, 0b1, None));
    data.extend(sequence_header_obu_with_lcr(3, 5));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/global-xlayer-map-missing-xlayer"),
        "report was: {report}"
    );
}

#[test]
fn lcr_reserved_bits_nonzero_is_warned() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu_with_nonzero_reserved());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .warnings()
            .any(|d| d.rule_id == "lcr/reserved-bits-nonzero"),
        "report was: {report}"
    );
}

#[test]
fn atlas_segment_mode_out_of_range_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(atlas_obu_bad_mode(31));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "atlas/segment-mode-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn lcr_global_id_zero_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(0, 0b1, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "lcr/global-id-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn lcr_empty_xlayer_map_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu(1, 0, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == "lcr/xlayer-map-empty"),
        "report was: {report}"
    );
}

#[test]
fn lcr_local_id_zero_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu(3, 0, 0, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == "lcr/local-id-zero"),
        "report was: {report}"
    );
}

#[test]
fn lcr_dependent_xlayers_flag_nonzero_is_warned() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu_with_dependent_flag());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .warnings()
            .any(|d| d.rule_id == "lcr/dependent-xlayers-flag-nonzero"),
        "report was: {report}"
    );
}

#[test]
fn lcr_config_idc_reserved_value_is_flagged() {
    let agg = super::lcr_msdo_cmvs::AggInfo {
        config_idc: 3,
        aggregate_level_idx: 0,
        max_tier_flag: 0,
        max_interop: 0,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(super::lcr_msdo_cmvs::global_lcr_obu_agreement(
        1,
        0b1,
        Some(agg),
        None,
        false,
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "lcr/config-idc-reserved"),
        "a reserved lcr_config_idc must fire; report was: {report}"
    );
}

#[test]
fn lcr_aggregate_level_idx_reserved_value_is_flagged() {
    let agg = super::lcr_msdo_cmvs::AggInfo {
        config_idc: 0,
        aggregate_level_idx: 22,
        max_tier_flag: 0,
        max_interop: 0,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(super::lcr_msdo_cmvs::global_lcr_obu_agreement(
        1,
        0b1,
        Some(agg),
        None,
        false,
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "lcr/aggregate-level-idx-reserved"),
        "a reserved lcr_aggregate_level_idx must fire; report was: {report}"
    );
}

#[test]
fn lcr_max_interop_reserved_value_is_flagged() {
    let agg = super::lcr_msdo_cmvs::AggInfo {
        config_idc: 0,
        aggregate_level_idx: 0,
        max_tier_flag: 0,
        max_interop: 3,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(super::lcr_msdo_cmvs::global_lcr_obu_agreement(
        1,
        0b1,
        Some(agg),
        None,
        false,
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "lcr/max-interop-reserved"),
        "a reserved lcr_max_interop must fire; report was: {report}"
    );
}

#[test]
fn lcr_aggregate_info_defined_values_are_accepted() {
    let agg = super::lcr_msdo_cmvs::AggInfo {
        config_idc: 2,
        aggregate_level_idx: 31,
        max_tier_flag: 0,
        max_interop: 15,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(super::lcr_msdo_cmvs::global_lcr_obu_agreement(
        1,
        0b1,
        Some(agg),
        None,
        false,
    ));
    let report = Validator::new(false).validate_bytes(&data);
    for rule in [
        "lcr/config-idc-reserved",
        "lcr/aggregate-level-idx-reserved",
        "lcr/max-interop-reserved",
    ] {
        assert!(
            !report.errors().any(|d| d.rule_id == rule),
            "defined § 6.8.4 aggregate values must not trip {rule}; report was: {report}"
        );
    }
}

#[test]
fn atlas_multistream_outside_global_xlayer_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(atlas_multistream_obu(3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "atlas/multistream-requires-global-xlayer"),
        "report was: {report}"
    );
}

#[test]
fn atlas_multistream_in_global_xlayer_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(atlas_multistream_obu(31));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "atlas/multistream-requires-global-xlayer"),
        "report was: {report}"
    );
}

#[test]
fn atlas_duplicate_input_stream_id_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(atlas_basic_duplicate_stream_obu(3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "atlas/duplicate-input-stream-id"),
        "report was: {report}"
    );
}

#[test]
fn atlas_multistream_duplicate_input_stream_id_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(atlas_multistream_duplicate_stream_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "atlas/duplicate-input-stream-id"),
        "report was: {report}"
    );
}

/// Wraps OPS payload bits with the extensible OBU tail (`obu_extension_flag = 0`
/// then `trailing_bits`).
pub(in crate::validator::tests) fn finish_extensible(mut bits: Bits) -> Vec<u8> {
    bits.bit(0); // obu_extension_flag
    bits.bit(1); // trailing_one_bit
    bits.align();
    bits.into_bytes()
}

/// Wraps non-extensible (BRT) payload bits with `trailing_bits` only.
pub(in crate::validator::tests) fn finish_non_extensible(mut bits: Bits) -> Vec<u8> {
    bits.bit(1); // trailing_one_bit
    bits.align();
    bits.into_bytes()
}

/// Appends one minimal global `operating_point_payload()`: a single included
/// extended layer (layer 0), no optional fields, `ops_mlayer_info_idc == 0` so no
/// PTL or mlayer info is coded. Writes a correct `ops_data_size`.
pub(in crate::validator::tests) fn append_minimal_global_payload(bits: &mut Bits) {
    let mut body = Bits::default();
    body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
    body.bit(0); // ops_initial_display_delay_present_flag
    body.f(0b1, 31); // ops_xlayer_map -> layer 0
    body.align();
    let body_bytes = (body.bits.len() / 8) as u32;
    bits.f(body_bytes, 8); // ops_data_size (single-byte leb128)
    bits.bits.extend_from_slice(&body.bits);
}

/// A global OPS OBU defining or resetting `ops_id` with `ops_cnt` minimal
/// operating points.
pub(in crate::validator::tests) fn global_ops_obu(
    reset: bool,
    ops_id: u32,
    ops_cnt: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(u8::from(reset)); // ops_reset_flag
    bits.f(ops_id, 4); // ops_id
    bits.f(ops_cnt, 3); // ops_cnt
    if ops_cnt > 0 {
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // ops_intent_present_flag
        bits.bit(0); // ops_ptl_present_flag
        bits.bit(0); // ops_color_info_present_flag
        bits.f(0, 2); // ops_mlayer_info_idc = 0
        for _ in 0..ops_cnt {
            append_minimal_global_payload(&mut bits);
        }
    }
    annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 31), &finish_extensible(bits))
}

/// A global OPS OBU (`ops_cnt == 1`, one included layer) with the given
/// `ops_mlayer_info_idc`. Only used with idc values (0 or 3) that code no mlayer
/// info for the layer.
pub(in crate::validator::tests) fn global_ops_idc_obu(idc: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(0); // ops_reset_flag
    bits.f(0, 4); // ops_id
    bits.f(1, 3); // ops_cnt
    bits.f(0, 4); // ops_priority
    bits.f(0, 7); // ops_intent
    bits.bit(0); // intent present
    bits.bit(0); // ptl present
    bits.bit(0); // color present
    bits.f(idc, 2); // ops_mlayer_info_idc
    append_minimal_global_payload(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 31), &finish_extensible(bits))
}

/// A global OPS OBU (`ops_cnt 1`, `idc 2`) with two included layers, where layer 1
/// inherits its mlayer info from `(embedded_ops_id, embedded_op_index)`. With
/// `embedded_ops_id == ops_id` this is a same-OPS reference; otherwise it resolves
/// against another OPS in the active store.
pub(in crate::validator::tests) fn global_ops_inherited_obu(
    ops_id: u32,
    embedded_ops_id: u32,
    embedded_op_index: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(0); // reset
    bits.f(ops_id, 4); // ops_id
    bits.f(1, 3); // ops_cnt = 1
    bits.f(0, 4); // priority
    bits.f(0, 7); // intent
    bits.bit(0); // intent present
    bits.bit(0); // ptl present
    bits.bit(0); // color present
    bits.f(2, 2); // ops_mlayer_info_idc = 2
    let mut body = Bits::default();
    body.bit(0); // decoder model present
    body.bit(0); // initial display delay present
    body.f(0b11, 31); // ops_xlayer_map -> layers 0 and 1
    body.bit(1); // layer 0: ops_mlayer_explicit_info_flag = 1
    body.f(0, 8); // layer 0: ops_mlayer_map = 0
    body.bit(0); // layer 1: ops_mlayer_explicit_info_flag = 0 -> inherited
    body.f(embedded_ops_id, 4); // layer 1: ops_embedded_ops_id
    body.f(embedded_op_index, 3); // layer 1: ops_embedded_op_index
    body.align();
    let body_bytes = (body.bits.len() / 8) as u32;
    bits.f(body_bytes, 8); // ops_data_size
    bits.bits.extend_from_slice(&body.bits);
    annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 31), &finish_extensible(bits))
}

/// A local OPS OBU on `xlayer` with `ops_cnt` minimal payloads and the given
/// `ops_reserved_2bits`. When `size_delta != 0`, the first payload's
/// `ops_data_size` is offset by `size_delta` to force a size mismatch.
pub(in crate::validator::tests) fn local_ops_obu(
    xlayer: u8,
    reset: bool,
    ops_id: u32,
    ops_cnt: u32,
    reserved_2bits: u32,
    ptl_present: bool,
    size_delta: i32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(u8::from(reset));
    bits.f(ops_id, 4);
    bits.f(ops_cnt, 3);
    if ops_cnt > 0 {
        bits.f(0, 4); // priority
        bits.f(0, 7); // intent
        bits.bit(0); // intent present
        bits.bit(u8::from(ptl_present)); // ptl present
        bits.bit(0); // color present
        bits.f(reserved_2bits, 2); // ops_reserved_2bits
        for index in 0..ops_cnt {
            let mut body = Bits::default();
            if ptl_present {
                body.f(0, 5); // seq_profile_idc
                body.f(0, 5); // level_idx
                body.bit(0); // tier_flag
                body.f(0, 3); // mlayer_count
                body.f(0b11, 2); // ops_ptl_reserved_2bits (nonzero)
            }
            body.bit(0); // decoder model present
            body.bit(0); // initial display delay present
            body.f(0, 8); // ops_mlayer_info(): ops_mlayer_map = 0
            body.align();
            let body_bytes = (body.bits.len() / 8) as i64;
            let declared = if index == 0 {
                (body_bytes + i64::from(size_delta)).max(0) as u32
            } else {
                body_bytes as u32
            };
            bits.f(declared, 8); // ops_data_size
            bits.bits.extend_from_slice(&body.bits);
        }
    }
    annex_b_obu_with_header(
        &layer_obu_header(18, 0, 0, xlayer),
        &finish_extensible(bits),
    )
}

/// An OPS-dependent BRT OBU on `xlayer` referencing `br_ops_id` with `br_ops_cnt`
/// operating points (no per-op times).
pub(in crate::validator::tests) fn brt_dependent_obu(
    xlayer: u8,
    br_ops_id: u32,
    br_ops_cnt: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(1); // br_ops_dependent_flag
    bits.f(br_ops_id, 4);
    bits.f(br_ops_cnt, 3);
    for _ in 0..br_ops_cnt {
        bits.bit(0); // br_decoder_model_present_op_flag = 0
    }
    annex_b_obu_with_header(
        &layer_obu_header(15, 0, 0, xlayer),
        &finish_non_extensible(bits),
    )
}

/// An extended-layer (non-OPS-dependent) BRT OBU on `xlayer`.
pub(in crate::validator::tests) fn brt_extended_layer_obu(xlayer: u8) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(0); // br_ops_dependent_flag = 0
    bits.rg(0, 4); // br_time
    annex_b_obu_with_header(
        &layer_obu_header(15, 0, 0, xlayer),
        &finish_non_extensible(bits),
    )
}

/// A local OPS OBU on `xlayer` (`ops_cnt == 1`) whose single operating point
/// carries explicit `ops_decoder_model_info()` with the given decoder/encoder
/// buffer delays (`§ 5.11.3`). `reset` sets `ops_reset_flag`.
pub(in crate::validator::tests) fn local_ops_obu_with_delays(
    xlayer: u8,
    reset: bool,
    ops_id: u32,
    decoder_delay: u32,
    encoder_delay: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(u8::from(reset)); // ops_reset_flag
    bits.f(ops_id, 4); // ops_id
    bits.f(1, 3); // ops_cnt
    bits.f(0, 4); // ops_priority
    bits.f(0, 7); // ops_intent
    bits.bit(0); // ops_intent_present_flag
    bits.bit(0); // ops_ptl_present_flag
    bits.bit(0); // ops_color_info_present_flag
    bits.f(0, 2); // ops_reserved_2bits
    let mut body = Bits::default();
    body.bit(1); // ops_decoder_model_info_for_this_op_present_flag
    body.uvlc(decoder_delay); // ops_decoder_buffer_delay
    body.uvlc(encoder_delay); // ops_encoder_buffer_delay
    body.bit(0); // ops_low_delay_mode_flag
    body.bit(0); // ops_initial_display_delay_present_flag
    body.f(0, 8); // ops_mlayer_info(): ops_mlayer_map = 0
    body.align();
    let body_bytes = (body.bits.len() / 8) as u32;
    bits.f(body_bytes, 8); // ops_data_size
    bits.bits.extend_from_slice(&body.bits);
    annex_b_obu_with_header(
        &layer_obu_header(18, 0, 0, xlayer),
        &finish_extensible(bits),
    )
}

/// A CLK frame OBU on `xlayer` whose first tile group's frame header references
/// `seq_header_id` directly (`cur_mfh_id == 0`), confirming activation and starting
/// a new coded video sequence for the layer (§ 7.3.6).
pub(in crate::validator::tests) fn clk_frame_for_xlayer(xlayer: u8, seq_header_id: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(1); // is_first_tile_group
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(seq_header_id); // seq_header_id_in_frame_header
    annex_b_obu_with_header(&layer_obu_header(4, 0, 0, xlayer), &bits.into_bytes())
}

pub(in crate::validator::tests) fn decoder_model_warning_count(
    report: &ValidationReport,
    rule: &str,
) -> usize {
    report.warnings().filter(|d| d.rule_id == rule).count()
}

pub(in crate::validator::tests) fn ops_error_count(report: &ValidationReport, rule: &str) -> usize {
    report.errors().filter(|d| d.rule_id == rule).count()
}
