// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// A full intra CLK frame whose `immediate_output_frame` (and therefore the
/// coded-frame-unit output classification, AV2 § 7.3.3) is decidable: the body
/// reaches `intra_structure_tail`, so the core parser settles the output flags
/// rather than stopping early. With `immediate_output == true` the frame is an
/// output coded frame; with `false` (and the sequence's monotonic_output_order)
/// it is a non-output coded frame. `first_tile_group` sets `is_first_tile_group`.
pub(in crate::validator::tests) fn clk_frame_decidable(
    first_tile_group: bool,
    immediate_output: bool,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(u8::from(first_tile_group)); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(u8::from(immediate_output)); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0); // structure + loop-filter cluster (no bits past)
    fb.bit(0); // tx_mode_select = 0
    fb.f(0, 2); // reduced_tx_set = 0
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// A `frame_core_seq` temporal unit (TD + activating sequence header for xlayer
/// 0) for the decidable-frame segmentation tests.
pub(in crate::validator::tests) fn seg_td_and_seq() -> Vec<u8> {
    td_and_frame_core_seq(FrameCoreSeq::base())
}

/// A bare buffer-removal-timing OBU at xlayer 0 (the payload is not parsed by
/// the segmenter; its role is fixed by the OBU type).
pub(in crate::validator::tests) fn brt_obu() -> Vec<u8> {
    annex_b_obu(BRT_HEADER, &[0x80])
}

pub(in crate::validator::tests) fn has_frame_unit_error(
    report: &ValidationReport,
    rule: &str,
) -> bool {
    report.errors().any(|d| d.rule_id == rule)
}

#[test]
fn frame_unit_first_tile_group_flag_zero_on_first_is_flagged() {
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(false, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/first-tile-group-flag"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_first_tile_group_flag_conformant_is_silent() {
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_adjacent_units_split_on_first_tile_group_flag_is_silent() {
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // unit 1: flag 1 (ok)
    data.extend(clk_frame_decidable(true, true)); // unit 2: flag 1 (new unit, ok)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "two back-to-back flag-1 units must split silently; report was: {report}"
    );
}

#[test]
fn frame_unit_back_to_back_multi_tile_units_are_silent() {
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // unit 1, first tile: flag 1
    data.extend(clk_frame_decidable(false, true)); // unit 1, non-first tile: flag 0
    data.extend(clk_frame_decidable(true, true)); // unit 2, first tile: flag 1 (split)
    data.extend(clk_frame_decidable(false, true)); // unit 2, non-first tile: flag 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "two back-to-back 1-then-0 units must be silent; report was: {report}"
    );
}

#[test]
fn frame_unit_undecidable_first_tile_group_stays_silent() {
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(REGULAR_TILE_GROUP_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "an undecidable unit must be silent; report was: {report}"
    );
}

#[test]
fn frame_unit_brt_multiplicity_in_non_output_unit_is_flagged() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6 for the bridge ref idx
        ..FrameCoreSeq::base()
    });
    data.extend(brt_obu());
    data.extend(brt_obu());
    let mut fb = Bits::default();
    fb.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    fb.f(0, 3); // bridge_frame_ref_idx == 0 (< NumRefFrames)
    data.extend(annex_b_obu(BRIDGE_HEADER, &fb.into_bytes()));
    data.extend(temporal_delimiter_obu()); // resolve the unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/buffer-removal-timing-multiplicity"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_brt_multiplicity_in_output_unit_is_conformant() {
    let mut data = seg_td_and_seq();
    data.extend(brt_obu());
    data.extend(brt_obu());
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // output coded frame
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_frame_unit_error(&report, "frame-unit/buffer-removal-timing-multiplicity"),
        "an output unit permits multiple BRT; report was: {report}"
    );
}

/// An `OBU_BRIDGE_FRAME` whose `bridge_frame_ref_idx == 0` (in range for
/// `NumRefFrames >= 1`). A bridge frame is always a non-output coded frame (mirror
/// line 470). Carries no `is_first_tile_group` delimiter.
pub(in crate::validator::tests) fn bridge_obu() -> Vec<u8> {
    let mut fb = Bits::default();
    fb.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    fb.f(0, 3); // bridge_frame_ref_idx == 0 (< NumRefFrames)
    annex_b_obu(BRIDGE_HEADER, &fb.into_bytes())
}

#[test]
fn frame_unit_brt_multiplicity_with_back_to_back_bridge_frames_is_flagged() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6 for the bridge ref idx
        ..FrameCoreSeq::base()
    });
    data.extend(brt_obu());
    data.extend(brt_obu());
    data.extend(bridge_obu()); // bridge coded frame (open)
    data.extend(bridge_obu()); // same-type continuation -> still non-output by type
    data.extend(temporal_delimiter_obu()); // resolve the unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/buffer-removal-timing-multiplicity"),
        "back-to-back bridge frames stay non-output by type, so the § 7.3.4 BRT bound \
         must still fire; report was: {report}"
    );
}

#[test]
fn celu_bridge_only_celu_fires_missing_output() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6 for the bridge ref idx
        ..FrameCoreSeq::base()
    });
    data.extend(bridge_obu());
    data.extend(temporal_delimiter_obu()); // resolve the CELU
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/missing-output-frame-unit"),
        "a bridge-only CELU is type-decided non-output with no output unit, so \
         missing-output-frame-unit must fire; report was: {report}"
    );
}

#[test]
fn celu_bridge_layer_with_output_layer_fires_non_output_without_output() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1))); // max_mlayer_id == 1
    let mut bridge = Bits::default();
    bridge.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    bridge.f(0, 3); // bridge_frame_ref_idx == 0
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(19, 0, 0, 0), // OBU_BRIDGE_FRAME, mlayer 0
        &bridge.into_bytes(),
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(12, 0, 1, 0), // OBU_REGULAR_SEF, mlayer 1
        &[0x80],
    ));
    data.extend(temporal_delimiter_obu()); // resolve the CELU
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/non-output-without-output"),
        "the bridge embedded layer has a non-output unit but no output unit, so \
         non-output-without-output must fire; report was: {report}"
    );
}

#[test]
fn frame_unit_suffix_metadata_before_coded_frame_is_flagged() {
    let mut data = seg_td_and_seq();
    let suffix_meta = metadata_short_payload(0x80, 0x04, &[]);
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 0),
        &suffix_meta,
    ));
    data.extend(clk_frame_decidable(true, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/suffix-metadata-before-coded-frame"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_duplicate_content_interpretation_is_flagged() {
    let mut data = seg_td_and_seq();
    data.extend(content_interpretation_obu(0, 0, None));
    data.extend(content_interpretation_obu(0, 0, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/duplicate-content-interpretation"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_ci_after_mfh_is_region_order_error() {
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(MFH_HEADER, &[0x80]));
    data.extend(content_interpretation_obu(0, 0, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/region-order"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_sef_single_obu_violation_is_flagged() {
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // SEF: the complete coded frame
    let mut rtg = Bits::default();
    rtg.bit(0); // is_first_tile_group == 0: continuation claim against the SEF
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &rtg.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/sef-single-obu"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_unreadable_tile_delimiter_after_sef_is_undecidable_and_silent() {
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // SEF: the complete coded frame
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "an unreadable tile delimiter after a SEF is undecidable and must not fire \
         sef-single-obu; report was: {report}"
    );
}

#[test]
fn frame_unit_unreadable_tile_delimiter_after_different_type_frame_is_undecidable_and_silent() {
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // CLK coded frame (open)
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &[])); // unreadable flag, different type
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "an unreadable tile delimiter after a different-type coded frame is undecidable \
         and must not fire mixed-coded-frame-types; report was: {report}"
    );
}

#[test]
fn frame_unit_sef_after_sef_splits_into_new_unit_silently() {
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(LEADING_SEF_HEADER, &[0x80])); // unit 1: SEF
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // unit 2: SEF (new unit)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "two back-to-back SEF units must split silently; report was: {report}"
    );
}

#[test]
fn frame_unit_mixed_coded_frame_types_is_flagged() {
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // CLK, output, decidable
    let mut rtg = Bits::default();
    rtg.bit(0); // is_first_tile_group == 0
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &rtg.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/mixed-coded-frame-types"),
        "report was: {report}"
    );
}

pub(in crate::validator::tests) const LEADING_TIP_HEADER: u8 = 13 << 2; // OBU_LEADING_TIP (0x34)
pub(in crate::validator::tests) const REGULAR_TIP_HEADER: u8 = 14 << 2; // OBU_REGULAR_TIP (0x38)

#[test]
fn frame_unit_no_delimiter_different_type_back_to_back_splits_silently() {
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(LEADING_TIP_HEADER, &[0x80])); // coded frame 1 (TIP 13)
    data.extend(annex_b_obu(REGULAR_TIP_HEADER, &[0x80])); // different type -> new unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "a different no-delimiter frame type after a completed coded frame must split \
         silently, not fire mixed-coded-frame-types; report was: {report}"
    );
}

#[test]
fn frame_unit_no_delimiter_same_type_back_to_back_is_undecidable_and_silent() {
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_TIP_HEADER, &[0x80])); // coded frame, TIP 14
    data.extend(annex_b_obu(REGULAR_TIP_HEADER, &[0x80])); // same type -> undecidable
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "a same-type no-delimiter continuation is undecidable and must be silent (no \
         mixed-types false positive); report was: {report}"
    );
}

#[test]
fn frame_unit_sef_after_tip_frame_splits_silently() {
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_TIP_HEADER, &[0x80])); // unit 1: TIP coded frame
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // unit 2: SEF (new unit)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "a SEF after a completed TIP coded frame must split silently (no \
         sef-single-obu / mixed-coded-frame-types); report was: {report}"
    );
}

#[test]
fn frame_unit_sef_after_bridge_frame_splits_silently() {
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6 for the bridge ref idx
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.uvlc(0); // seq_header_id_in_frame_header (bridge infers cur_mfh_id == 0)
    fb.f(0, 3); // bridge_frame_ref_idx == 0 (< NumRefFrames)
    data.extend(annex_b_obu(BRIDGE_HEADER, &fb.into_bytes())); // unit 1: bridge coded frame
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // unit 2: SEF (new unit)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "a SEF after a completed bridge coded frame must split silently (no \
         sef-single-obu / mixed-coded-frame-types); report was: {report}"
    );
}

#[test]
fn frame_unit_mixed_coded_frame_types_still_flags_tile_group_continuation() {
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_TIP_HEADER, &[0x80])); // TIP coded frame
    let mut rtg = Bits::default();
    rtg.bit(0); // is_first_tile_group == 0: explicit non-first-tile continuation claim
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &rtg.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/mixed-coded-frame-types"),
        "a flag-0 tile group continuing a mismatched TIP coded frame must still fire \
         mixed-coded-frame-types; report was: {report}"
    );
}

#[test]
fn frame_unit_unreadable_suffix_metadata_after_coded_frame_keeps_unit_intact() {
    let mut data = seg_td_and_seq();
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // valid coded frame
    data.extend(annex_b_obu_with_header(&layer_obu_header(8, 0, 0, 0), &[]));
    let suffix_meta = metadata_short_payload(0x80, 0x04, &[]);
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 0),
        &suffix_meta,
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "an unreadable-suffix metadata after a coded frame must keep the unit intact \
         (region-blind), not cascade a missing-coded-frame finding; report was: {report}"
    );
}

#[test]
fn frame_unit_ci_in_second_frame_unit_is_flagged() {
    let mut data = seg_td_and_seq();
    data.extend(content_interpretation_obu(0, 0, None)); // first unit's CI (ok)
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // closes the first unit
    data.extend(content_interpretation_obu(0, 0, None)); // second unit's CI (violation)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/ci-not-in-first-frame-unit"),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_ci_in_later_temporal_layer_unit_is_flagged() {
    let mut data = seg_td_and_seq();
    data.extend(content_interpretation_obu(0, 0, None)); // tlayer 0: first unit's CI
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // tlayer 0: closes first unit
    let mut ci_bits = Bits::default();
    ci_bits.f(0, 2); // ci_scan_type_idc
    ci_bits.bit(0); // ci_color_description_present_flag
    ci_bits.bit(0); // ci_chroma_sample_position_present_flag
    ci_bits.bit(0); // ci_aspect_ratio_info_present_flag
    ci_bits.bit(0); // ci_timing_info_present_flag
    ci_bits.f(0, 2); // ci_reserved_2bit
    ci_bits.bit(0); // obu_extension_flag
    ci_bits.bit(1); // trailing_one_bit
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(24, 1, 0, 0), // CI at tlayer 1
        &ci_bits.into_bytes(),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_frame_unit_error(&report, "frame-unit/ci-not-in-first-frame-unit"),
        "a CI in a later temporal-layer unit of the same embedded layer must fire; \
         report was: {report}"
    );
}

#[test]
fn frame_unit_padding_is_position_free() {
    let mut data = seg_td_and_seq();
    data.extend(brt_obu());
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(25, 0, 0, 0),
        &[0x80],
    )); // padding
    data.extend(clk_frame_decidable(true, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "padding is position-free; report was: {report}"
    );
}

#[test]
fn frame_unit_conformant_output_unit_is_silent() {
    let mut data = seg_td_and_seq();
    data.extend(content_interpretation_obu(0, 0, None));
    data.extend(annex_b_obu(MFH_HEADER, &[0x80]));
    data.extend(brt_obu());
    data.extend(clk_frame_decidable(true, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-unit/")),
        "report was: {report}"
    );
}

#[test]
fn frame_unit_trailing_head_run_at_temporal_unit_end_is_flagged() {
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // unit 1 (complete)
    data.extend(brt_obu()); // starts a head-only unit 2
    data.extend(temporal_delimiter_obu()); // seals unit 2 with no coded frame
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-unit/missing-coded-frame"),
        "a head-only unit sealed by a temporal delimiter must error; report was: {report}"
    );
}

#[test]
fn frame_unit_trailing_head_run_at_stream_end_is_a_warning() {
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // unit 1 (complete)
    data.extend(brt_obu()); // head-only unit 2, no coded frame, stream ends
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .warnings()
            .any(|d| d.rule_id == "frame-unit/missing-coded-frame"),
        "a head-only unit at the end of the bitstream must warn; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-unit/missing-coded-frame"),
        "the end-of-stream head-only unit must not be an error; report was: {report}"
    );
}

#[test]
fn frame_unit_complete_unit_at_stream_end_has_no_missing_coded_frame() {
    let mut data = seg_td_and_seq();
    data.extend(brt_obu());
    data.extend(clk_frame_decidable(true, true)); // completes the unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "frame-unit/missing-coded-frame"),
        "a complete unit must not report a missing coded frame; report was: {report}"
    );
}

#[test]
fn obu_order_global_hls_after_metadata_suffix_is_flagged() {
    let mut data = temporal_delimiter_obu();
    let suffix_meta = metadata_short_payload(0x80, 0x04, &[]);
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &suffix_meta,
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(16, 0, 0, 31),
        &[],
    )); // global LCR
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-order/global-hls-after-metadata-suffix"),
        "report was: {report}"
    );
}

#[test]
fn obu_order_non_global_hls_before_coded_layer_is_flagged() {
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // frame region for xlayer 0
    data.extend(annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 0), &[])); // LCR xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "obu-order/non-global-hls-before-coded-layer"
                && d.spec_section.as_deref() == Some("7.3.6")
        }),
        "report was: {report}"
    );
}

#[test]
fn obu_order_non_global_hls_header_before_frame_is_conformant() {
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 0), &[])); // LCR xlayer 0
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // frame region after
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "obu-order/non-global-hls-before-coded-layer"),
        "report was: {report}"
    );
}

#[test]
fn obu_order_non_global_hls_before_earlier_xlayer_frame_is_flagged() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(15, 0, 0, 0),
        &[0x80],
    )); // BRT xlayer 0
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(15, 0, 0, 1),
        &[0x80],
    )); // BRT xlayer 1
    data.extend(annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 0), &[])); // LCR xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-order/non-global-hls-before-coded-layer"),
        "an LCR for an earlier xlayer whose frame region began must fire even after a \
         later xlayer's frame region; report was: {report}"
    );
}
