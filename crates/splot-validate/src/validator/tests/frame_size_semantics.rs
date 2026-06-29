// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn validator_frame_header_copy_record_cleared_by_sef_opening_new_frame() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group()); // records NumFrameHeaderBits for the triple
    data.extend(conformant_sef_same_triple()); // SEF: own coded frame, same triple
    let mut mismatched = complete_intra_clk_frame_header_body().drain_bits();
    mismatched[2] ^= 1;
    data.extend(clk_non_first_tile_group(&mismatched));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-header/copy-bits-")),
        "a SEF opening a new coded frame must clear the prior tile frame's record so a \
         later flag-0 tile group does not pair against it; report was: {report}"
    );
    assert!(
        has_frame_unit_error(&report, "frame-unit/sef-single-obu"),
        "the flag-0 tile group continuing the SEF coded frame must still fire \
         sef-single-obu; report was: {report}"
    );
}

/// A multi-frame header OBU (type 3) at xlayer 0 with `mfh_seq_header_id` 0 and
/// `mfhId` 1 (`mfh_id_minus_1` 0), carrying an `mfh_frame_size_present_flag`
/// payload whose `mfh_frame_width_minus_1` / `mfh_frame_height_minus_1` derive the
/// given `FrameWidth` / `FrameHeight` (AV2 § 5.7). No segmentation info, no
/// deblocking update. A `cur_mfh_id == 1` frame with `frame_size_override_flag == 0`
/// then derives its FrameWidth/FrameHeight from this record (mirror :5767).
pub(in crate::validator::tests) fn mfh_obu_with_frame_size(width: u32, height: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(0); // mfh_seq_header_id 0
    bits.uvlc(0); // mfh_id_minus_1 -> mfhId = 1
    bits.bit(1); // mfh_frame_size_present_flag
    bits.f(8 - 1, 4); // mfh_frame_width_bits_minus_1 -> 8-bit width field
    bits.f(8 - 1, 4); // mfh_frame_height_bits_minus_1 -> 8-bit height field
    bits.f(width - 1, 8); // mfh_frame_width_minus_1 -> FrameWidth = width
    bits.f(height - 1, 8); // mfh_frame_height_minus_1 -> FrameHeight = height
    bits.bit(0); // mfh_deblocking_filter_update
    bits.bit(0); // mfh_seg_info_present_flag -> fully parsed
    bits.bit(0); // obu_extension_flag = 0
    bits.bit(1); // trailing_one_bit
    annex_b_obu(0x0C, &bits.into_bytes())
}

/// A multi-frame header OBU (type 3) at xlayer 0 with `mfh_seq_header_id` 0 and
/// `mfhId` 1, with `mfh_frame_size_present_flag == 0` (no stored dimensions). AV2
/// § 5.18.2 (:4101) infers the default dims to the sequence maxima at consumption,
/// so the §6.17.2 `mfh_frame_width_minus_1 <= max_frame_width_minus_1` bound has no
/// stored value to compare and is trivially satisfied.
pub(in crate::validator::tests) fn mfh_obu_without_frame_size() -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(0); // mfh_seq_header_id 0
    bits.uvlc(0); // mfh_id_minus_1 -> mfhId = 1
    bits.bit(0); // mfh_frame_size_present_flag == 0 (no stored dims)
    bits.bit(0); // mfh_deblocking_filter_update
    bits.bit(0); // mfh_seg_info_present_flag -> fully parsed
    bits.bit(0); // obu_extension_flag = 0
    bits.bit(1); // trailing_one_bit
    annex_b_obu(0x0C, &bits.into_bytes())
}

/// A `cur_mfh_id == 1`, `frame_size_override_flag == 0` CLK frame (AV2 § 5.18.2)
/// whose FrameWidth/FrameHeight come from the resolved MFH record's default
/// dimensions (no explicit width/height bits). `col_increment_bits` is the number
/// of `tile_info()` column-increment bits the derived FrameWidth requires.
pub(in crate::validator::tests) fn mfh_backed_clk_default_size(col_increment_bits: u32) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(1); // cur_mfh_id == 1 (resolves through the MFH; no seq id field follows)
    fb.bit(0); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag == 0 -> MFH default dims
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, col_increment_bits);
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// A `cur_mfh_id == 1`, `frame_size_override_flag == 1` CLK frame (AV2 § 5.18.2)
/// whose FrameWidth/FrameHeight come from explicit `frame_width_minus_1` /
/// `frame_height_minus_1` bits (`width` / `height`), ignoring the resolved MFH's
/// stored default dimensions. `col_increment_bits` is the number of `tile_info()`
/// column-increment bits the overridden FrameWidth requires. The explicit width
/// field is the sequence `frame_width_bits` wide (8 for [`FrameCoreSeq::base`]).
pub(in crate::validator::tests) fn mfh_backed_clk_override_size(
    width: u32,
    height: u32,
    col_increment_bits: u32,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(1); // cur_mfh_id == 1 (resolves through the MFH; no seq id field follows)
    fb.bit(0); // immediate_output_frame
    fb.bit(1); // frame_size_override_flag == 1 -> explicit dims override the MFH
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.f(width - 1, 8); // frame_width_minus_1 (f(frame_width_bits == 8))
    fb.f(height - 1, 8); // frame_height_minus_1 (f(frame_height_bits == 8))
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, col_increment_bits);
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

#[test]
fn validator_flags_mfh_stored_frame_size_exceeds_sequence_max_on_override() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(256, 8)); // stored dims 256x8 (> max 16x16)
    data.extend(mfh_backed_clk_override_size(16, 16, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "the in-range override must not fire the derived frame-size check; report was: {report}"
    );
}

#[test]
fn validator_accepts_mfh_stored_frame_size_within_sequence_max_on_override() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(16, 16));
    data.extend(mfh_backed_clk_override_size(16, 16, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
}

#[test]
fn validator_does_not_flag_mfh_omitted_frame_size_on_override() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_without_frame_size());
    data.extend(mfh_backed_clk_override_size(16, 16, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "an omitted MFH size infers the maxima and must stay silent; report was: {report}"
    );
}

#[test]
fn validator_does_not_double_fire_mfh_frame_size_on_default_path() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(256, 8)); // stored dims 256x8 (> max 16x16)
    data.extend(mfh_backed_clk_default_size(1));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max")
            .count(),
        1,
        "the stored-MFH check is the single home for the default path; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "the derived check must defer to the stored-MFH check on the default path; \
         report was: {report}"
    );
}

#[test]
fn validator_does_not_flag_mfh_stored_frame_size_when_mfh_unresolvable() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_backed_clk_override_size(16, 16, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "an unresolvable MFH must keep the silent behavior; report was: {report}"
    );
}

#[test]
fn validator_flags_mfh_default_frame_size_exceeds_sequence_max() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(256, 8));
    data.extend(mfh_backed_clk_default_size(1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_mfh_default_frame_size_within_sequence_max() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(16, 16));
    data.extend(mfh_backed_clk_default_size(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
}

#[test]
fn validator_does_not_flag_mfh_default_frame_size_when_mfh_unresolvable() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_backed_clk_default_size(1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "an unresolvable MFH must keep the early-stop behavior; report was: {report}"
    );
}

#[test]
fn validator_flags_mfh_stored_frame_size_exceeds_sequence_max_on_truncated_frame() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(256, 8)); // stored dims 256x8 (> max 16x16)
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1)); // truncated after the prefix
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "a truncated frame referencing an oversized-stored-dims MFH must still fire \
         the §6.17.2 stored-dims check; report was: {report}"
    );
}

#[test]
fn validator_accepts_mfh_within_sequence_max_on_truncated_frame() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(16, 16)); // stored dims 16x16 == max
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1)); // truncated after the prefix
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "a conformant-MFH truncated frame must stay silent; report was: {report}"
    );
}

#[test]
fn validator_flags_both_frame_size_rules_when_override_repeats_mfh_out_of_range_dims() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(256, 8)); // stored dims 256x8 (> max 16x16)
    data.extend(mfh_backed_clk_override_size(256, 8, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "the §6.17.2 stored-MFH check must fire; report was: {report}"
    );
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "an override==1 frame's explicit out-of-range dims are a separate §6.17.4.1 \
         violation that must fire even when they equal the stored MFH dims; \
         report was: {report}"
    );
}

pub(in crate::validator::tests) const OLK_HEADER: u8 = 0x14;
pub(in crate::validator::tests) const RTG_HEADER: u8 = 0x1c;

#[test]
fn validator_flags_frame_to_refresh_out_of_range() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6
        enable_short_refresh_frame_flags: true,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
    fb.bit(0); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag (default dims)
    fb.f(0, 1); // order_hint
    fb.bit(1); // has_refresh_frame_flags
    fb.f(6, 3); // frame_to_refresh == 6 (CeilLog2(6) == 3) -> refresh = 1 << 6
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-to-refresh-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_reserved_ref_long_term_id() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // restricted_prediction_switch
    fb.f(1, 3); // num_key_ref_frames == 1
    fb.f(15, 4); // ref_long_term_id[0] == (1 << 4) - 1 (reserved)
    data.extend(annex_b_obu(RAS_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/ref-long-term-id-reserved"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_zero_refresh_on_deferred_output() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_size_override_flag (default dims)
    fb.f(0, 1); // order_hint
    fb.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8) == 0
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(OLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/refresh-frame-flags-zero-on-deferred-output"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_still_picture_non_key_frame() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        still_picture: true,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY (not KEY_FRAME)
    fb.bit(1); // immediate_output_frame == 1 (isolate the frame-type violation)
    fb.bit(0); // frame_size_override_flag (default dims)
    fb.f(0, 1); // order_hint
    fb.f(1, 8); // refresh_frame_flags f(8) == 1 (nonzero, not all-slots)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/still-picture-requires-key-frame"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_intra_only_refresh_all_slots() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 1, // NumRefFrames == 2
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
    fb.bit(1); // immediate_output_frame == 1
    fb.bit(0); // frame_size_override_flag (default dims)
    fb.f(0, 1); // order_hint
    fb.f(0b11, 2); // refresh_frame_flags f(NumRefFrames == 2) == all slots
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/intra-only-refresh-all-slots"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_in_range_frame_to_refresh() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5,
        enable_short_refresh_frame_flags: true,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
    fb.bit(0); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint
    fb.bit(1); // has_refresh_frame_flags
    fb.f(5, 3); // frame_to_refresh == 5 (< NumRefFrames 6)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-to-refresh-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_non_reserved_ref_long_term_id() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // restricted_prediction_switch
    fb.f(1, 3); // num_key_ref_frames == 1
    fb.f(14, 4); // ref_long_term_id[0] == 14 (not reserved)
    data.extend(annex_b_obu(RAS_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/ref-long-term-id-reserved"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_nonzero_refresh_on_deferred_output() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint
    fb.f(1, 8); // refresh_frame_flags f(8) == 1 (nonzero)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(OLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/refresh-frame-flags-zero-on-deferred-output"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_still_picture_key_frame() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        still_picture: true,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // immediate_output_frame == 1
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/still-picture-requires-key-frame"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_intra_only_partial_refresh() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 1, // NumRefFrames == 2
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
    fb.bit(1); // immediate_output_frame == 1
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint
    fb.f(0b01, 2); // refresh_frame_flags == 1 (not all slots)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/intra-only-refresh-all-slots"),
        "report was: {report}"
    );
}

/// Appends a CLK frame-header bit fixture from the activation prefix through
/// `disable_cdf_update`, with `frame_size_override_flag == 1` and the given
/// dimensions, leaving `fb` positioned at `tile_info()` (AV2 § 5.18.2). The
/// dimension fields are `f(width_bits)` / `f(height_bits)`
/// (`frame_*_bits_minus_1 + 1` from the sequence).
pub(in crate::validator::tests) fn clk_frame_until_tile_info(
    fb: &mut Bits,
    width: u32,
    height: u32,
    bits: (u32, u32),
) {
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame
    fb.bit(1); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.f(width - 1, bits.0); // frame_width_minus_1
    fb.f(height - 1, bits.1); // frame_height_minus_1
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
}

/// Appends the post-`tile_info()` § 5.18.2 structures with every optional read
/// disabled: `base_q_idx` f(9) (§ 5.18.6.1), `segmentation_enabled = 0`
/// (§ 5.18.7.1), `using_qmatrix = 0` (§ 5.18.6.2), `delta_q_present = 0`
/// (§ 5.18.7.8); the lossless tail then reads no bits.
pub(in crate::validator::tests) fn quant_seg_tail(fb: &mut Bits) {
    fb.f(100, 9); // base_q_idx f(9) (10-bit sequence)
    fb.bit(0); // segmentation_enabled
    fb.bit(0); // using_qmatrix
    fb.bit(0); // delta_q_present
}

/// Encodes `ns(n)` value `0` (AV2 § 4.11.6): `w = FloorLog2(n) + 1`,
/// `m = (1 << w) - n`; `0 < m` always holds, so the encoding is `f(0, w - 1)`.
pub(in crate::validator::tests) fn ns_zero(fb: &mut Bits, n: u32) {
    let w = 32 - n.leading_zeros();
    fb.f(0, w - 1);
}
