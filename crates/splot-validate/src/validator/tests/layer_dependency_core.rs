// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// A base-layer (xlayer 0) sequence header payload with `max_tlayer_id == 1`,
/// `max_mlayer_id == 1`, and a signaled mlayer dependency map that *clears*
/// `MLayerDependencyMap[1][0]` (embedded layer 1 does not depend on layer 0),
/// overriding the § 5.4.1 lower-triangular default fill.
pub(in crate::validator::tests) fn sequence_header_payload_mlayer_dep_cleared(
    seq_header_id: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(seq_header_id);
    bits.f(0, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    bits.f(0, 5); // seq_level_idx
    bits.uvlc(0); // chroma_format_idc
    bits.uvlc(0); // bit_depth_idc
    bits.f(0, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(1, 2); // max_tlayer_id
    bits.f(1, 3); // max_mlayer_id
    bits.f(1, 1); // seq_max_mlayer_cnt_minus_1 -> SeqMaxMlayerCnt = 2 (layers 0, 1)
    bits.bit(1); // monotonic_output_order_flag
    bits.f(3, 4); // frame_width_bits_minus_1
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(15, 4); // max_frame_width_minus_1
    bits.f(7, 4); // max_frame_height_minus_1
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    bits.bit(1); // mlayer_dependency_present_flag
    bits.bit(1); // mlayer_dependency_map -> MLayerDependencyMap[1][1] = 1
    bits.bit(0); // mlayer_dependency_map -> MLayerDependencyMap[1][0] = 0
    bits.bit(0); // tlayer_dependency_present_flag
    append_non_single_child_configs(&mut bits);
    bits.into_bytes()
}

/// A local OPS OBU at `xlayer` (`ops_cnt == 1`) whose single payload carries an
/// explicit `ops_mlayer_info()` with the given maps. `tlayer_maps` holds one
/// `ops_tlayer_map` per set bit of `mlayer_map`, in ascending set-bit order.
pub(in crate::validator::tests) fn local_ops_mlayer_obu(
    xlayer: u8,
    ops_id: u32,
    mlayer_map: u8,
    tlayer_maps: &[u8],
) -> Vec<u8> {
    assert_eq!(mlayer_map.count_ones() as usize, tlayer_maps.len());
    let mut bits = Bits::default();
    bits.bit(0); // ops_reset_flag
    bits.f(ops_id, 4); // ops_id
    bits.f(1, 3); // ops_cnt = 1
    bits.f(0, 4); // ops_priority
    bits.f(0, 7); // ops_intent
    bits.bit(0); // ops_intent_present_flag
    bits.bit(0); // ops_ptl_present_flag
    bits.bit(0); // ops_color_info_present_flag
    bits.f(0, 2); // ops_reserved_2bits
    let mut body = Bits::default();
    body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
    body.bit(0); // ops_initial_display_delay_present_flag
    body.f(u32::from(mlayer_map), 8); // ops_mlayer_map
    for &tlayer_map in tlayer_maps {
        body.f(u32::from(tlayer_map), 4); // ops_tlayer_map
    }
    body.align();
    let body_bytes = (body.bits.len() / 8) as u32;
    bits.f(body_bytes, 8); // ops_data_size
    bits.bits.extend_from_slice(&body.bits);
    annex_b_obu_with_header(
        &layer_obu_header(18, 0, 0, xlayer),
        &finish_extensible(bits),
    )
}

/// A global OPS OBU (`ops_cnt == 1`, `ops_mlayer_info_idc == 1`) whose single
/// included extended layer `target_xlayer` carries an explicit
/// `ops_mlayer_info()` with the given maps.
pub(in crate::validator::tests) fn global_ops_explicit_obu(
    ops_id: u32,
    target_xlayer: u8,
    mlayer_map: u8,
    tlayer_maps: &[u8],
) -> Vec<u8> {
    assert_eq!(mlayer_map.count_ones() as usize, tlayer_maps.len());
    let mut bits = Bits::default();
    bits.bit(0); // ops_reset_flag
    bits.f(ops_id, 4); // ops_id
    bits.f(1, 3); // ops_cnt = 1
    bits.f(0, 4); // ops_priority
    bits.f(0, 7); // ops_intent
    bits.bit(0); // ops_intent_present_flag
    bits.bit(0); // ops_ptl_present_flag
    bits.bit(0); // ops_color_info_present_flag
    bits.f(1, 2); // ops_mlayer_info_idc = 1 -> explicit mlayer info per layer
    let mut body = Bits::default();
    body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
    body.bit(0); // ops_initial_display_delay_present_flag
    body.f(1u32 << target_xlayer, 31); // ops_xlayer_map
    body.f(u32::from(mlayer_map), 8); // ops_mlayer_map
    for &tlayer_map in tlayer_maps {
        body.f(u32::from(tlayer_map), 4); // ops_tlayer_map
    }
    body.align();
    let body_bytes = (body.bits.len() / 8) as u32;
    bits.f(body_bytes, 8); // ops_data_size
    bits.bits.extend_from_slice(&body.bits);
    annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 31), &finish_extensible(bits))
}

/// Appends a § 5.8.8 `lcr_embedded_layer_info()` block: per set bit `j` of
/// `mlayer_map` (ascending) a TEXTURE layer with the given `lcr_tlayer_map`, an
/// all-zero `lcr_dependent_layer_map` (when `j > 0`), the same-resolution flag
/// set, and the per-iteration `byte_alignment()`.
pub(in crate::validator::tests) fn append_lcr_embedded_layer_info(
    bits: &mut Bits,
    mlayer_map: u8,
    tlayer_maps: &[u8],
) {
    assert_eq!(mlayer_map.count_ones() as usize, tlayer_maps.len());
    bits.f(u32::from(mlayer_map), 8); // lcr_mlayer_map
    let mut next_tlayer = 0usize;
    for j in 0u8..8 {
        if mlayer_map & (1u8 << j) == 0 {
            continue;
        }
        bits.f(u32::from(tlayer_maps[next_tlayer]), 4); // lcr_tlayer_map
        next_tlayer += 1;
        bits.f(0, 8); // lcr_layer_type = TEXTURE_LAYER
        bits.f(0, 8); // lcr_view_type = VIEW_UNSPECIFIED
        if j > 0 {
            bits.f(0, u32::from(j)); // lcr_dependent_layer_map
        }
        bits.bit(1); // lcr_same_sh_max_resolution_flag
        bits.align(); // byte_alignment()
    }
}

/// Appends a § 5.8.8 `lcr_embedded_layer_info()` block where each set bit `j` of
/// `mlayer_map` (ascending) carries an explicit `lcr_max_expected_width` /
/// `lcr_max_expected_height` (`lcr_same_sh_max_resolution_flag == 0`) from `dims[k]`.
pub(in crate::validator::tests) fn append_lcr_embedded_layer_info_with_dims(
    bits: &mut Bits,
    mlayer_map: u8,
    tlayer_maps: &[u8],
    dims: &[(u32, u32)],
) {
    assert_eq!(mlayer_map.count_ones() as usize, tlayer_maps.len());
    assert_eq!(mlayer_map.count_ones() as usize, dims.len());
    bits.f(u32::from(mlayer_map), 8); // lcr_mlayer_map
    let mut next = 0usize;
    for j in 0u8..8 {
        if mlayer_map & (1u8 << j) == 0 {
            continue;
        }
        bits.f(u32::from(tlayer_maps[next]), 4); // lcr_tlayer_map
        bits.f(0, 8); // lcr_layer_type = TEXTURE_LAYER
        bits.f(0, 8); // lcr_view_type = VIEW_UNSPECIFIED
        if j > 0 {
            bits.f(0, u32::from(j)); // lcr_dependent_layer_map
        }
        bits.bit(0); // lcr_same_sh_max_resolution_flag = 0 -> explicit dims follow
        bits.uvlc(dims[next].0); // lcr_max_expected_width
        bits.uvlc(dims[next].1); // lcr_max_expected_height
        next += 1;
        bits.align(); // byte_alignment()
    }
}

/// A local LCR OBU at `xlayer` carrying embedded-layer info with explicit per-layer
/// `lcr_max_expected_width`/`height` (`same_sh_max_resolution_flag == 0`).
pub(in crate::validator::tests) fn local_lcr_obu_with_expected_dims(
    xlayer: u8,
    local_id: u32,
    mlayer_map: u8,
    tlayer_maps: &[u8],
    dims: &[(u32, u32)],
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 3); // lcr_global_id
    bits.f(local_id, 3); // lcr_local_id
    bits.bit(0); // lcr_profile_tier_level_info_present_flag
    bits.bit(0); // lcr_local_atlas_id_present_flag
    bits.f(0, 3); // reserved_zero_3bits
    bits.f(0, 5); // lcr_local_reserved_zero_5bits
    bits.bit(0); // lcr_rep_info_present_flag
    bits.bit(0); // lcr_xlayer_purpose_present_flag
    bits.bit(0); // lcr_xlayer_color_info_present_flag
    bits.bit(1); // lcr_embedded_layer_info_present_flag
    bits.align(); // byte_alignment()
    append_lcr_embedded_layer_info_with_dims(&mut bits, mlayer_map, tlayer_maps, dims);
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, xlayer), &bits.into_bytes())
}

/// A local LCR OBU at `xlayer` (`lcr_global_id == 0`) carrying embedded-layer
/// info with the given maps.
pub(in crate::validator::tests) fn local_lcr_obu_with_embedded(
    xlayer: u8,
    local_id: u32,
    mlayer_map: u8,
    tlayer_maps: &[u8],
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 3); // lcr_global_id
    bits.f(local_id, 3); // lcr_local_id
    bits.bit(0); // lcr_profile_tier_level_info_present_flag
    bits.bit(0); // lcr_local_atlas_id_present_flag
    bits.f(0, 3); // reserved_zero_3bits
    bits.f(0, 5); // lcr_local_reserved_zero_5bits
    bits.bit(0); // lcr_rep_info_present_flag
    bits.bit(0); // lcr_xlayer_purpose_present_flag
    bits.bit(0); // lcr_xlayer_color_info_present_flag
    bits.bit(1); // lcr_embedded_layer_info_present_flag
    bits.align(); // byte_alignment()
    append_lcr_embedded_layer_info(&mut bits, mlayer_map, tlayer_maps);
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, xlayer), &bits.into_bytes())
}

/// A global LCR OBU whose `lcr_xlayer_map` includes only `target_xlayer` and
/// whose single global payload carries embedded-layer info with the given maps.
pub(in crate::validator::tests) fn global_lcr_obu_with_embedded(
    global_id: u32,
    target_xlayer: u8,
    mlayer_map: u8,
    tlayer_maps: &[u8],
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(global_id, 3); // lcr_global_config_record_id
    bits.f(1u32 << target_xlayer, 31); // lcr_xlayer_map
    bits.bit(0); // lcr_aggregate_info_present_flag
    bits.bit(0); // lcr_seq_profile_tier_level_info_present_flag
    bits.bit(1); // lcr_global_payload_present_flag
    bits.bit(0); // lcr_dependent_xlayers_flag
    bits.bit(0); // lcr_global_atlas_id_present_flag
    bits.f(0, 7); // lcr_global_purpose_id
    bits.bit(0); // lcr_doh_constraint_flag
    bits.bit(0); // lcr_enforce_tile_alignment_flag
    bits.f(0, 3); // reserved_zero_3bits
    bits.f(0, 5); // lcr_global_reserved_zero_5bits
    let mut body = Bits::default();
    body.bit(0); // lcr_rep_info_present_flag
    body.bit(0); // lcr_xlayer_purpose_present_flag
    body.bit(0); // lcr_xlayer_color_info_present_flag
    body.bit(1); // lcr_embedded_layer_info_present_flag
    body.align(); // byte_alignment()
    append_lcr_embedded_layer_info(&mut body, mlayer_map, tlayer_maps);
    let body_bytes = (body.bits.len() / 8) as u32;
    bits.f(body_bytes, 8); // lcr_global_data_size (single-byte leb128)
    bits.bits.extend_from_slice(&body.bits);
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
}

/// A multi-frame header OBU at `(tlayer, mlayer)` on xlayer 0 referencing
/// `seq_header_id` (`mfh_id_minus_1 == 0`, so `mfhId == 1`).
pub(in crate::validator::tests) fn multi_frame_header_obu_with_layers(
    seq_header_id: u32,
    tlayer: u8,
    mlayer: u8,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(seq_header_id); // mfh_seq_header_id
    bits.uvlc(0); // mfh_id_minus_1 -> mfhId = 1
    bits.bit(0); // mfh_frame_size_present_flag
    bits.bit(0); // mfh_deblocking_filter_update
    bits.bit(0); // mfh_seg_info_present_flag
    bits.bit(0); // obu_extension_flag = 0
    bits.bit(1); // trailing_one_bit
    annex_b_obu_with_header(&layer_obu_header(3, tlayer, mlayer, 0), &bits.into_bytes())
}

/// A CLK frame-bearing OBU at `(tlayer, mlayer)` on xlayer 0 whose frame header
/// references the multi-frame header `cur_mfh_id`.
pub(in crate::validator::tests) fn frame_obu_mfh_ref_with_layers(
    tlayer: u8,
    mlayer: u8,
    cur_mfh_id: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(1); // is_first_tile_group
    bits.uvlc(cur_mfh_id); // cur_mfh_id > 0
    annex_b_obu_with_header(&layer_obu_header(4, tlayer, mlayer, 0), &bits.into_bytes())
}

/// A global OPS OBU (`ops_cnt == 1`, `ops_mlayer_info_idc == 2`) whose single
/// included extended layer 0 *inherits* its mlayer info from
/// `(embedded_ops_id, embedded_op_index)`. Cross-OPS inheritance keeps the
/// layer-0 entry legal (same-OPS layer-0 inheritance is always out of range).
pub(in crate::validator::tests) fn global_ops_layer0_inherited_obu(
    ops_id: u32,
    embedded_ops_id: u32,
    embedded_op_index: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(0); // ops_reset_flag
    bits.f(ops_id, 4); // ops_id
    bits.f(1, 3); // ops_cnt = 1
    bits.f(0, 4); // ops_priority
    bits.f(0, 7); // ops_intent
    bits.bit(0); // ops_intent_present_flag
    bits.bit(0); // ops_ptl_present_flag
    bits.bit(0); // ops_color_info_present_flag
    bits.f(2, 2); // ops_mlayer_info_idc = 2 -> explicit-or-inherited per layer
    let mut body = Bits::default();
    body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
    body.bit(0); // ops_initial_display_delay_present_flag
    body.f(0b1, 31); // ops_xlayer_map -> layer 0 only
    body.bit(0); // layer 0: ops_mlayer_explicit_info_flag = 0 -> inherited
    body.f(embedded_ops_id, 4); // ops_embedded_ops_id
    body.f(embedded_op_index, 3); // ops_embedded_op_index
    body.align();
    let body_bytes = (body.bits.len() / 8) as u32;
    bits.f(body_bytes, 8); // ops_data_size
    bits.bits.extend_from_slice(&body.bits);
    annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 31), &finish_extensible(bits))
}

#[test]
fn ops_mlayer_dependency_missing_is_flagged() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "ops/mlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn ops_tlayer_dependency_missing_is_flagged() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(local_ops_mlayer_obu(0, 0, 0b1, &[0b10]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "ops/tlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn ops_dependency_closed_maps_are_not_flagged() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(local_ops_mlayer_obu(0, 0, 0b11, &[0b11, 0b11]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "ops/mlayer-dependency-missing")
            && !has_error(&report, "ops/tlayer-dependency-missing"),
        "dependency-closed maps must be silent; report was: {report}"
    );
}

#[test]
fn ops_dependency_respects_signaled_mlayer_map() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_mlayer_dep_cleared(0),
    ));
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "ops/mlayer-dependency-missing"),
        "a signaled non-dependency must not be flagged; report was: {report}"
    );
}

#[test]
fn ops_before_sequence_header_is_checked_once_at_activation() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1)));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "ops/mlayer-dependency-missing"),
        1,
        "report was: {report}"
    );
}

#[test]
fn ops_without_activated_sequence_header_is_not_checked() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "ops/mlayer-dependency-missing")
            && !has_error(&report, "ops/tlayer-dependency-missing"),
        "no activated sequence header means no agreement check; report was: {report}"
    );
}

#[test]
fn ops_dependency_check_suppressed_under_external_sequence_headers() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(5)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "ops/mlayer-dependency-missing"),
        "external sequence headers must suppress the check; report was: {report}"
    );
}

#[test]
fn ops_inherited_mlayer_info_is_not_dependency_checked() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(global_ops_explicit_obu(1, 0, 0b10, &[0b1])); // flagged once
    data.extend(global_ops_layer0_inherited_obu(0, 1, 0)); // inherits OPS 1 op 0
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "ops/mlayer-dependency-missing"),
        1,
        "only the source OPS's explicit maps may be flagged; report was: {report}"
    );
    assert!(
        !has_error(&report, "ops/tlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn ops_global_explicit_entry_mlayer_dependency_missing_is_flagged() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(global_ops_explicit_obu(0, 0, 0b10, &[0b1]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "ops/mlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn ops_dependency_not_duplicated_across_reactivation() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "ops/mlayer-dependency-missing"),
        1,
        "report was: {report}"
    );
}

#[test]
fn lcr_mlayer_dependency_missing_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/mlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn lcr_tlayer_dependency_missing_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu_with_embedded(5, 3, 0b1, &[0b10]));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(1, 0, 0, 3),
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/tlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn lcr_dependency_closed_maps_are_not_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b11, &[0b11, 0b11]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "dependency-closed maps must be silent; report was: {report}"
    );
}

#[test]
fn lcr_without_seq_lcr_reference_is_not_checked() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "an unreferenced LCR must not be checked; report was: {report}"
    );
}

#[test]
fn lcr_max_expected_dims_exceed_sequence_max_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_expected_dims(
        0,
        5,
        0b1,
        &[0b1],
        &[(17, 8)],
    ));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // frame-confirms the activation
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/max-expected-dims-exceed-sequence-max"
                && d.spec_section.as_deref() == Some("6.8.9")
        }),
        "report was: {report}"
    );
}

#[test]
fn lcr_max_expected_dims_at_sequence_max_boundary_is_silent() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_expected_dims(
        0,
        5,
        0b1,
        &[0b1],
        &[(16, 8)],
    ));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/max-expected-dims-exceed-sequence-max"),
        "an at-maximum expected dimension must not fire; report was: {report}"
    );
}

#[test]
fn lcr_max_expected_dims_without_frame_confirmation_is_silent() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_expected_dims(
        0,
        5,
        0b1,
        &[0b1],
        &[(17, 8)],
    ));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/max-expected-dims-exceed-sequence-max"),
        "an unconfirmed activation must not fire; report was: {report}"
    );
}

#[test]
fn lcr_max_expected_dims_suppressed_under_external_provided() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet};
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_expected_dims(
        0,
        5,
        0b1,
        &[0b1],
        &[(17, 8)],
    ));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/max-expected-dims-exceed-sequence-max"),
        "a Provided external-HLS mode must suppress the §6.8.9 bound; report was: {report}"
    );
}

#[test]
fn lcr_dependency_diagnostic_points_at_lcr_obu() {
    let td = temporal_delimiter_obu();
    let lcr = local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]);
    let seq_start = (td.len() + lcr.len()) as u64;
    let mut data = td;
    data.extend(lcr);
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    let offsets: Vec<_> = report
        .errors()
        .filter(|d| d.rule_id == "lcr/mlayer-dependency-missing")
        .map(|d| d.byte_offset)
        .collect();
    assert!(
        matches!(offsets.as_slice(), [Some(offset)] if offset.get() < seq_start),
        "exactly one diagnostic pointing at the LCR OBU (before byte {seq_start}) was \
         expected; report was: {report}"
    );
}

#[test]
fn lcr_dependency_not_duplicated_across_reactivation() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "lcr/mlayer-dependency-missing"),
        1,
        "report was: {report}"
    );
}

#[test]
fn frame_mfh_mlayer_dependency_missing_is_flagged() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(multi_frame_header_obu_with_layers(0, 0, 1));
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "frame-header/mfh-mlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn frame_mfh_tlayer_dependency_missing_is_flagged() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(multi_frame_header_obu_with_layers(0, 1, 0));
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "frame-header/mfh-tlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn frame_mfh_satisfied_layer_dependencies_are_not_flagged() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(multi_frame_header_obu(0));
    data.extend(frame_obu_mfh_ref_with_layers(1, 1, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "frame-header/mfh-mlayer-dependency-missing")
            && !has_error(&report, "frame-header/mfh-tlayer-dependency-missing"),
        "satisfied dependencies must be silent; report was: {report}"
    );
}

#[test]
fn frame_mfh_unavailable_is_not_layer_checked() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 2));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "hls/unavailable-multi-frame-header"),
        "report was: {report}"
    );
    assert!(
        !has_error(&report, "frame-header/mfh-mlayer-dependency-missing")
            && !has_error(&report, "frame-header/mfh-tlayer-dependency-missing"),
        "an unresolved MFH must not be layer-checked; report was: {report}"
    );
}

#[test]
fn frame_mfh_external_sequence_header_is_not_layer_checked() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(multi_frame_header_obu_with_layers(5, 0, 1));
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(5)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "frame-header/mfh-mlayer-dependency-missing")
            && !has_error(&report, "frame-header/mfh-tlayer-dependency-missing"),
        "an externally resolved sequence header must not be layer-checked; report was: {report}"
    );
}

#[test]
fn frame_mfh_unresolvable_sequence_header_is_not_layer_checked() {
    let mut data = temporal_delimiter_obu();
    data.extend(multi_frame_header_obu_with_layers(5, 0, 1));
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "hls/unavailable-sequence-header"),
        "report was: {report}"
    );
    assert!(
        !has_error(&report, "frame-header/mfh-mlayer-dependency-missing")
            && !has_error(&report, "frame-header/mfh-tlayer-dependency-missing"),
        "an unresolvable sequence header must not be layer-checked; report was: {report}"
    );
}

/// A `film_grain_obu()` at `(tlayer, mlayer)` on xlayer 0 (film-grain OBU type 23) with the
/// given `update_flags` and `fgm_chroma_idc`, one minimal model per set update-flag bit. The
/// layered header lets a test record `FgmMLayerId` / `FgmTLayerId` at a layer the
/// grain-applying frame does not depend on.
fn film_grain_obu_at_layer(update_flags: u32, chroma_idc: u32, tlayer: u8, mlayer: u8) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(update_flags, 8); // fgm_update_flags
    bits.uvlc(chroma_idc); // fgm_chroma_idc
    for _ in 0..update_flags.count_ones() {
        append_minimal_film_grain_model(&mut bits);
    }
    bits.bit(1); // trailing_one_bit (FG is non-extensible)
    bits.align();
    annex_b_obu_with_header(&layer_obu_header(23, tlayer, mlayer, 0), &bits.into_bytes())
}

#[test]
fn frame_film_grain_mlayer_dependency_missing_is_flagged() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        max_mlayer_id: 1,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(film_grain_obu_at_layer(1 << 0, 0, 0, 1)); // slot 0 defined at mlayer 1
    data.extend(sef_with_applied_grain(0)); // frame at mlayer 0 references fgm_id 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "frame-header/film-grain-mlayer-dependency-missing"),
        "a film-grain model at an undepended embedded layer must fire \
         film-grain-mlayer-dependency-missing; report was: {report}"
    );
}

#[test]
fn frame_film_grain_tlayer_dependency_missing_is_flagged() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        max_tlayer_id: 1,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(film_grain_obu_at_layer(1 << 0, 0, 1, 0)); // slot 0 defined at tlayer 1
    data.extend(sef_with_applied_grain(0)); // frame at tlayer 0 references fgm_id 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "frame-header/film-grain-tlayer-dependency-missing"),
        "a film-grain model at an undepended temporal layer must fire \
         film-grain-tlayer-dependency-missing; report was: {report}"
    );
}

#[test]
fn frame_film_grain_chroma_idc_mismatch_is_flagged() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(film_grain_obu_bytes(1 << 0, 2)); // slot 0, fgm_chroma_idc = 2
    data.extend(sef_with_applied_grain(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "frame-header/film-grain-chroma-idc-mismatch"),
        "a film-grain model whose FgmChromaIdc differs from chroma_format_idc must fire \
         film-grain-chroma-idc-mismatch; report was: {report}"
    );
}

#[test]
fn frame_film_grain_satisfied_constraints_are_silent() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        max_mlayer_id: 1,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(film_grain_obu_bytes(1 << 0, 0)); // slot 0, chroma 0, layer 0
    data.extend(sef_with_applied_grain(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "frame-header/film-grain-mlayer-dependency-missing")
            && !has_error(&report, "frame-header/film-grain-tlayer-dependency-missing")
            && !has_error(&report, "frame-header/film-grain-chroma-idc-mismatch"),
        "satisfied film-grain layer-dependency / chroma constraints must be silent; \
         report was: {report}"
    );
}

#[test]
fn frame_film_grain_unavailable_is_not_layer_checked() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(sef_with_applied_grain(0)); // references fgm_id 0, never defined
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "frame-header/film-grain-model-unavailable"),
        "report was: {report}"
    );
    assert!(
        !has_error(&report, "frame-header/film-grain-mlayer-dependency-missing")
            && !has_error(&report, "frame-header/film-grain-tlayer-dependency-missing")
            && !has_error(&report, "frame-header/film-grain-chroma-idc-mismatch"),
        "an unavailable film-grain model must not be layer-checked; report was: {report}"
    );
}

#[test]
fn frame_film_grain_layer_checks_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        max_mlayer_id: 1,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(film_grain_obu_at_layer(1 << 0, 2, 0, 1)); // mlayer 1 + chroma 2: both violate
    data.extend(sef_with_applied_grain(0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "frame-header/film-grain-mlayer-dependency-missing")
            && !has_error(&report, "frame-header/film-grain-tlayer-dependency-missing")
            && !has_error(&report, "frame-header/film-grain-chroma-idc-mismatch"),
        "a Provided external-HLS mode must suppress the film-grain layer-dependency / chroma \
         checks; report was: {report}"
    );
}

#[test]
fn frame_film_grain_intra_tail_path_is_checked() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(film_grain_obu_bytes(1 << 0, 2)); // slot 0, fgm_chroma_idc = 2
    data.extend(super::rap_replay_foundation::clk_intra_frame_applying_grain(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "frame-header/film-grain-chroma-idc-mismatch"),
        "the §6.17.10.1 constraints must be checked on the intra-tail (CLK) film_grain_config \
         path, not only the SEF path; report was: {report}"
    );
}

#[test]
fn frame_film_grain_out_of_range_chroma_idc_does_not_double_fire_mismatch() {
    let seq = FrameCoreSeq {
        film_grain_params_present: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(film_grain_obu_bytes(1 << 0, 4)); // slot 0, fgm_chroma_idc = 4 (> 3)
    data.extend(sef_with_applied_grain(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "film-grain/chroma-idc-out-of-range"),
        "the §6.13 out-of-range check owns the malformed value; report was: {report}"
    );
    assert!(
        !has_error(&report, "frame-header/film-grain-chroma-idc-mismatch"),
        "an out-of-range fgm_chroma_idc must not also fire the §6.17.10.1 mismatch; \
         report was: {report}"
    );
}
