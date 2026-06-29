// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// Parameters for a § 6.8.5 PTL-bearing sequence header.
#[derive(Clone, Copy)]
pub(in crate::validator::tests) struct SeqPtl {
    pub(in crate::validator::tests) seq_header_id: u32,
    pub(in crate::validator::tests) seq_lcr_id: u32,
    pub(in crate::validator::tests) profile: u32,
    pub(in crate::validator::tests) level: u32,
    /// `seq_tier` — only signalled (and so only != Main) when `level > 3`.
    pub(in crate::validator::tests) tier: u32,
    /// `max_mlayer_id`; `SeqMaxMlayerCnt == max_mlayer_id + 1`.
    pub(in crate::validator::tests) max_mlayer_id: u32,
}

/// A sequence header carrying the given § 6.8.5 PTL fields (`max_tlayer_id == 1`),
/// otherwise identical to [`sequence_header_payload_with_lcr`]. `seq_tier` is only
/// signalled in the bitstream when `seq_level_idx > 3` (§ 5.4.1).
pub(in crate::validator::tests) fn seq_header_ptl_payload(p: SeqPtl) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(p.seq_header_id);
    bits.f(p.profile, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    bits.f(p.level, 5); // seq_level_idx
    if p.level > 3 {
        bits.bit(p.tier as u8); // seq_tier
    }
    bits.uvlc(0); // chroma_format_idc
    bits.uvlc(0); // bit_depth_idc
    bits.f(p.seq_lcr_id, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(1, 2); // max_tlayer_id
    bits.f(p.max_mlayer_id, 3); // max_mlayer_id
    if p.max_mlayer_id > 0 {
        bits.f(p.max_mlayer_id, ceil_log2_u32(p.max_mlayer_id + 1)); // seq_max_mlayer_cnt_minus_1
    }
    bits.bit(1); // monotonic_output_order_flag
    bits.f(3, 4); // frame_width_bits_minus_1
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(15, 4); // max_frame_width_minus_1 -> width 16
    bits.f(7, 4); // max_frame_height_minus_1 -> height 8
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    if p.max_mlayer_id > 0 {
        bits.bit(0); // mlayer_dependency_present_flag
    }
    bits.bit(0); // tlayer_dependency_present_flag (max_tlayer_id == 1 > 0)
    append_non_single_child_configs(&mut bits);
    annex_b_obu(0x04, &bits.into_bytes())
}

/// Parameters for a § 6.8.8 rep-info-bearing sequence header.
#[derive(Clone, Copy)]
pub(in crate::validator::tests) struct SeqRep {
    pub(in crate::validator::tests) seq_header_id: u32,
    pub(in crate::validator::tests) seq_lcr_id: u32,
    /// `max_frame_width_minus_1` (`f(4)`), so the width is this + 1.
    pub(in crate::validator::tests) width_minus_1: u32,
    /// `max_frame_height_minus_1` (`f(4)`).
    pub(in crate::validator::tests) height_minus_1: u32,
    pub(in crate::validator::tests) chroma_format_idc: u32,
    pub(in crate::validator::tests) bit_depth_idc: u32,
    /// `seq_cropping_window_present_flag` and the four offsets when present.
    pub(in crate::validator::tests) cropping: Option<(u32, u32, u32, u32)>,
}

/// The raw `sequence_header_obu()` payload bytes carrying the given § 6.8.8 rep-info
/// fields (no embedded layers, `max_tlayer_id == 1`, `max_mlayer_id == 0`).
pub(in crate::validator::tests) fn seq_header_rep_payload_bytes(p: SeqRep) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(p.seq_header_id);
    bits.f(0, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    bits.f(0, 5); // seq_level_idx
    bits.uvlc(p.chroma_format_idc); // chroma_format_idc
    bits.uvlc(p.bit_depth_idc); // bit_depth_idc
    bits.f(p.seq_lcr_id, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(1, 2); // max_tlayer_id
    bits.f(0, 3); // max_mlayer_id == 0
    bits.bit(1); // monotonic_output_order_flag
    bits.f(3, 4); // frame_width_bits_minus_1 (4-bit dims)
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(p.width_minus_1, 4); // max_frame_width_minus_1
    bits.f(p.height_minus_1, 4); // max_frame_height_minus_1
    match p.cropping {
        Some((left, right, top, bottom)) => {
            bits.bit(1); // seq_cropping_window_present_flag
            bits.uvlc(left); // seq_cropping_win_left_offset
            bits.uvlc(right);
            bits.uvlc(top);
            bits.uvlc(bottom);
        }
        None => bits.bit(0), // seq_cropping_window_present_flag
    }
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    bits.bit(0); // tlayer_dependency_present_flag (max_tlayer_id == 1)
    append_non_single_child_configs(&mut bits);
    bits.into_bytes()
}

/// A sequence header carrying the given § 6.8.8 rep-info fields (no embedded layers,
/// `max_tlayer_id == 1`, `max_mlayer_id == 0`), on extended layer 0.
pub(in crate::validator::tests) fn seq_header_rep_payload(p: SeqRep) -> Vec<u8> {
    annex_b_obu(0x04, &seq_header_rep_payload_bytes(p))
}

/// As [`seq_header_rep_payload`], but on the given `xlayer` (a § 6.2.2 base-layer
/// sequence header — `tlayer == 0`, `mlayer == 0` — that can activate seq id `p` for
/// that extended layer).
pub(in crate::validator::tests) fn seq_header_rep_obu_for_xlayer(xlayer: u8, p: SeqRep) -> Vec<u8> {
    let payload = seq_header_rep_payload_bytes(p);
    if xlayer == 0 {
        annex_b_obu(0x04, &payload)
    } else {
        annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
    }
}

/// A local LCR OBU at `xlayer` carrying `lcr_seq_profile_tier_level_info(xlayer)`
/// with the given declared maxima (no rep info, no embedded info).
pub(in crate::validator::tests) fn local_lcr_obu_with_ptl(
    xlayer: u8,
    local_id: u32,
    max_profile: u32,
    max_level: u32,
    max_tier: u32,
    max_mlayer_count: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 3); // lcr_global_id
    bits.f(local_id, 3); // lcr_local_id
    bits.bit(1); // lcr_profile_tier_level_info_present_flag
    bits.bit(0); // lcr_local_atlas_id_present_flag
    bits.f(max_profile, 5); // lcr_seq_profile_idc
    bits.f(max_level, 5); // lcr_max_level_idx
    bits.bit(max_tier as u8); // lcr_tier_flag
    bits.f(max_mlayer_count, 3); // lcr_max_mlayer_count
    bits.f(0, 2); // lsptli_reserved_2bits
    bits.f(0, 3); // reserved_zero_3bits (no atlas)
    bits.f(0, 5); // lcr_local_reserved_zero_5bits
    bits.bit(0); // lcr_rep_info_present_flag
    bits.bit(0); // lcr_xlayer_purpose_present_flag
    bits.bit(0); // lcr_xlayer_color_info_present_flag
    bits.bit(0); // lcr_embedded_layer_info_present_flag
    bits.align(); // byte_alignment()
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, xlayer), &bits.into_bytes())
}

/// Appends an `lcr_rep_info()` body with the given width/height, optional
/// format info `(bit_depth, chroma)`, and optional cropping window
/// `(left, right, top, bottom)`.
pub(in crate::validator::tests) fn append_lcr_rep_info(
    bits: &mut Bits,
    width: u32,
    height: u32,
    format: Option<(u32, u32)>,
    cropping: Option<(u32, u32, u32, u32)>,
) {
    bits.uvlc(width); // lcr_max_pic_width
    bits.uvlc(height); // lcr_max_pic_height
    bits.bit(u8::from(format.is_some())); // lcr_format_info_present_flag
    bits.bit(u8::from(cropping.is_some())); // lcr_cropping_window_present_flag
    if let Some((bit_depth, chroma)) = format {
        bits.uvlc(bit_depth); // lcr_bit_depth_idc
        bits.uvlc(chroma); // lcr_chroma_format_idc
    }
    if let Some((left, right, top, bottom)) = cropping {
        bits.uvlc(left); // lcr_cropping_win_left_offset
        bits.uvlc(right);
        bits.uvlc(top);
        bits.uvlc(bottom);
    }
}

/// A local LCR OBU at `xlayer` carrying `lcr_rep_info(0, xId)` (no PTL, no embedded
/// info).
pub(in crate::validator::tests) fn local_lcr_obu_with_rep_info(
    xlayer: u8,
    local_id: u32,
    width: u32,
    height: u32,
    format: Option<(u32, u32)>,
    cropping: Option<(u32, u32, u32, u32)>,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 3); // lcr_global_id
    bits.f(local_id, 3); // lcr_local_id
    bits.bit(0); // lcr_profile_tier_level_info_present_flag
    bits.bit(0); // lcr_local_atlas_id_present_flag
    bits.f(0, 3); // reserved_zero_3bits
    bits.f(0, 5); // lcr_local_reserved_zero_5bits
    bits.bit(1); // lcr_rep_info_present_flag
    bits.bit(0); // lcr_xlayer_purpose_present_flag
    bits.bit(0); // lcr_xlayer_color_info_present_flag
    bits.bit(0); // lcr_embedded_layer_info_present_flag
    append_lcr_rep_info(&mut bits, width, height, format, cropping);
    bits.align(); // byte_alignment()
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, xlayer), &bits.into_bytes())
}

/// A global LCR OBU whose `lcr_xlayer_map` includes only `target_xlayer` and whose
/// single global payload carries `lcr_rep_info(1, xId)` with the given fields (no
/// PTL, no embedded info).
pub(in crate::validator::tests) fn global_lcr_obu_with_rep_info(
    global_id: u32,
    target_xlayer: u8,
    width: u32,
    height: u32,
    format: Option<(u32, u32)>,
    cropping: Option<(u32, u32, u32, u32)>,
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
    body.bit(1); // lcr_rep_info_present_flag
    body.bit(0); // lcr_xlayer_purpose_present_flag
    body.bit(0); // lcr_xlayer_color_info_present_flag
    body.bit(0); // lcr_embedded_layer_info_present_flag
    append_lcr_rep_info(&mut body, width, height, format, cropping);
    body.align(); // byte_alignment()
    let body_bytes = (body.bits.len() / 8) as u32;
    debug_assert!(
        body_bytes < 128,
        "lcr_global_data_size must fit a single-byte leb128"
    );
    bits.f(body_bytes, 8); // lcr_global_data_size (single-byte leb128)
    bits.bits.extend_from_slice(&body.bits);
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
}

/// A global LCR OBU whose `lcr_xlayer_map` includes only `target_xlayer` and that
/// carries `lcr_seq_profile_tier_level_info(target_xlayer)` with the given maxima
/// (no payload).
pub(in crate::validator::tests) fn global_lcr_obu_with_ptl(
    global_id: u32,
    target_xlayer: u8,
    max_profile: u32,
    max_level: u32,
    max_tier: u32,
    max_mlayer_count: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(global_id, 3); // lcr_global_config_record_id
    bits.f(1u32 << target_xlayer, 31); // lcr_xlayer_map
    bits.bit(0); // lcr_aggregate_info_present_flag
    bits.bit(1); // lcr_seq_profile_tier_level_info_present_flag
    bits.bit(0); // lcr_global_payload_present_flag
    bits.bit(0); // lcr_dependent_xlayers_flag
    bits.bit(0); // lcr_global_atlas_id_present_flag
    bits.f(0, 7); // lcr_global_purpose_id
    bits.bit(0); // lcr_doh_constraint_flag
    bits.bit(0); // lcr_enforce_tile_alignment_flag
    bits.f(0, 3); // reserved_zero_3bits
    bits.f(0, 5); // lcr_global_reserved_zero_5bits
    bits.f(max_profile, 5); // lcr_seq_profile_idc
    bits.f(max_level, 5); // lcr_max_level_idx
    bits.bit(max_tier as u8); // lcr_tier_flag
    bits.f(max_mlayer_count, 3); // lcr_max_mlayer_count
    bits.f(0, 2); // lsptli_reserved_2bits
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
}

#[test]
fn lcr_ptl_level_exceeds_max_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 8,
        tier: 0,
        max_mlayer_id: 0,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/ptl-level-exceeds-max"),
        "report was: {report}"
    );
}

#[test]
fn lcr_ptl_profile_exceeds_max_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 1, 31, 0, 7));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 3,
        level: 0,
        tier: 0,
        max_mlayer_id: 0,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/ptl-profile-exceeds-max"),
        "report was: {report}"
    );
}

#[test]
fn lcr_ptl_tier_exceeds_max_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 31, 0, 7));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 5,
        tier: 1,
        max_mlayer_id: 0,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/ptl-tier-exceeds-max"),
        "report was: {report}"
    );
}

#[test]
fn lcr_ptl_mlayer_count_exceeds_max_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 31, 0, 1));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 0,
        tier: 0,
        max_mlayer_id: 1,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/ptl-mlayer-count-exceeds-max"),
        "report was: {report}"
    );
}

#[test]
fn lcr_ptl_equality_passes() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 2, 5, 1, 2));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 2,
        level: 5,
        tier: 1,
        max_mlayer_id: 1,
    }));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/ptl-profile-exceeds-max")
            && !has_error(&report, "lcr/ptl-level-exceeds-max")
            && !has_error(&report, "lcr/ptl-tier-exceeds-max")
            && !has_error(&report, "lcr/ptl-mlayer-count-exceeds-max"),
        "equality must pass; report was: {report}"
    );
}

#[test]
fn lcr_ptl_absent_info_compares_nothing() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu(0, 0, 5, None)); // no PTL, no rep info
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 4,
        level: 20,
        tier: 0,
        max_mlayer_id: 0,
    }));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/ptl-level-exceeds-max")
            && !has_error(&report, "lcr/ptl-profile-exceeds-max"),
        "absent PTL info must compare nothing; report was: {report}"
    );
}

#[test]
fn lcr_ptl_unconfirmed_activation_is_silent_then_fires_on_frame() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 0, // no LCR -> not violating
        profile: 0,
        level: 0,
        tier: 0,
        max_mlayer_id: 0,
    }));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 1,
        seq_lcr_id: 5,
        profile: 0,
        level: 8, // > lcr_max_level_idx 4
        tier: 0,
        max_mlayer_id: 0,
    }));
    let staged = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&staged, "lcr/ptl-level-exceeds-max"),
        "an unconfirmed activation must be silent; report was: {staged}"
    );
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1));
    let confirmed = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&confirmed, "lcr/ptl-level-exceeds-max"),
        "the frame-confirmed activation must fire; report was: {confirmed}"
    );
}

#[test]
fn lcr_ptl_global_record_ceiling_is_checked() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu_with_ptl(5, 0, 0, 4, 0, 1));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 8,
        tier: 0,
        max_mlayer_id: 0,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/ptl-level-exceeds-max"),
        "report was: {report}"
    );
}

#[test]
fn lcr_ptl_not_duplicated_across_reactivation() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 8,
        tier: 0,
        max_mlayer_id: 0,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "lcr/ptl-level-exceeds-max"),
        1,
        "report was: {report}"
    );
}

#[test]
fn lcr_ptl_redefinition_rechecks_affected_layer() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 31, 0, 7));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 8,
        tier: 0,
        max_mlayer_id: 0,
    }));
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 7)); // redefinition: ceiling 4
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 8,
        tier: 0,
        max_mlayer_id: 0,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "lcr/ptl-level-exceeds-max"),
        1,
        "the redefinition's lowered ceiling must re-check exactly once; report was: {report}"
    );
}

#[test]
fn lcr_ptl_diagnostic_points_at_lcr_obu() {
    let td = temporal_delimiter_obu();
    let lcr = local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1);
    let seq_start = (td.len() + lcr.len()) as u64;
    let mut data = td;
    data.extend(lcr);
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 8,
        tier: 0,
        max_mlayer_id: 0,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    let offsets: Vec<_> = report
        .errors()
        .filter(|d| d.rule_id == "lcr/ptl-level-exceeds-max")
        .map(|d| d.byte_offset)
        .collect();
    assert!(
        matches!(offsets.as_slice(), [Some(offset)] if offset.get() < seq_start),
        "the diagnostic must point at the LCR OBU (before byte {seq_start}); report: {report}"
    );
}

#[test]
fn lcr_ptl_suppressed_under_external_hls_provided() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 8, // > lcr_max_level_idx 4
        tier: 0,
        max_mlayer_id: 0,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    assert!(
        has_error(
            &Validator::new(false).validate_bytes(&data),
            "lcr/ptl-level-exceeds-max"
        ),
        "the in-band ceiling violation must fire under Disabled"
    );
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "lcr/ptl-level-exceeds-max"),
        "an empty Provided set must suppress the association-dependent PTL ceiling \
         check; report was: {report}"
    );
}

#[test]
fn lcr_ptl_suppressed_under_ops_only_external_hls_provided() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 8, // > lcr_max_level_idx 4
        tier: 0,
        max_mlayer_id: 0,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(0, 3),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "lcr/ptl-level-exceeds-max"),
        "an OPS-only Provided set must suppress the association-dependent PTL ceiling \
         check; report was: {report}"
    );
}
