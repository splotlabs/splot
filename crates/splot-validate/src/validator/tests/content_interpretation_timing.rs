// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[derive(Clone, Copy)]
pub(in crate::validator::tests) struct CiTiming {
    pub(in crate::validator::tests) display_tick: u32,
    pub(in crate::validator::tests) time_scale: u32,
    pub(in crate::validator::tests) equal_picture_interval: bool,
    pub(in crate::validator::tests) num_ticks_minus_1: u32,
}

/// Builds an `OBU_CONTENT_INTERPRETATION` (type 24) at obu_xlayer_id 0 /
/// obu_mlayer_id `mlayer`, with all optional branches cleared except the
/// requested timing, plus the §5.2.1 extensible payload tail.
pub(in crate::validator::tests) fn content_interpretation_obu(
    mlayer: u8,
    reserved_2bit: u32,
    timing: Option<CiTiming>,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 2); // ci_scan_type_idc
    bits.bit(0); // ci_color_description_present_flag
    bits.bit(0); // ci_chroma_sample_position_present_flag
    bits.bit(0); // ci_aspect_ratio_info_present_flag
    bits.bit(u8::from(timing.is_some())); // ci_timing_info_present_flag
    bits.f(reserved_2bit, 2); // ci_reserved_2bit
    if let Some(t) = timing {
        bits.f(t.display_tick, 32);
        bits.f(t.time_scale, 32);
        bits.bit(u8::from(t.equal_picture_interval));
        if t.equal_picture_interval {
            bits.uvlc(t.num_ticks_minus_1);
        }
    }
    bits.bit(0); // obu_extension_flag = 0
    bits.bit(1); // trailing_one_bit
    annex_b_obu_with_header(&layer_obu_header(24, 0, mlayer, 0), &bits.into_bytes())
}

/// Temporal delimiter + an activating sequence header for xlayer 0 that allows
/// embedded layers 0 and 1, then two content-interpretation OBUs at embedded
/// layers 0 and 1.
pub(in crate::validator::tests) fn stream_with_two_ci_layers(
    a: Option<CiTiming>,
    b: Option<CiTiming>,
) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, a));
    data.extend(content_interpretation_obu(1, 0, b));
    data
}

pub(in crate::validator::tests) const BASE_TIMING: CiTiming = CiTiming {
    display_tick: 1000,
    time_scale: 30000,
    equal_picture_interval: true,
    num_ticks_minus_1: 1,
};

#[test]
fn ci_matching_timing_across_embedded_layers_is_accepted() {
    let data = stream_with_two_ci_layers(Some(BASE_TIMING), Some(BASE_TIMING));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("sequence-header/timing-")),
        "report was: {report}"
    );
    assert!(
        conformant_apart_from_header_only_celu(&report),
        "report was: {report}"
    );
}

#[test]
fn ci_different_display_tick_across_embedded_layers_is_flagged() {
    let other = CiTiming {
        display_tick: 2000,
        ..BASE_TIMING
    };
    let data = stream_with_two_ci_layers(Some(BASE_TIMING), Some(other));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-header/timing-display-tick-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn ci_different_time_scale_across_embedded_layers_is_flagged() {
    let other = CiTiming {
        time_scale: 60000,
        ..BASE_TIMING
    };
    let data = stream_with_two_ci_layers(Some(BASE_TIMING), Some(other));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-header/timing-time-scale-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn ci_different_equal_picture_interval_across_embedded_layers_is_flagged() {
    let other = CiTiming {
        equal_picture_interval: false,
        ..BASE_TIMING
    };
    let data = stream_with_two_ci_layers(Some(BASE_TIMING), Some(other));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-header/timing-equal-picture-interval-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn ci_different_num_ticks_across_embedded_layers_is_flagged() {
    let other = CiTiming {
        num_ticks_minus_1: 4,
        ..BASE_TIMING
    };
    let data = stream_with_two_ci_layers(Some(BASE_TIMING), Some(other));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-header/timing-num-ticks-mismatch"),
        "report was: {report}"
    );
}

/// Two temporal units: a CI with `BASE_TIMING` at embedded layer 0 in temporal
/// unit 1, then a CI with a differing `time_scale` at embedded layer 1 in
/// temporal unit 2 (same extended layer 0, no CLK).
pub(in crate::validator::tests) fn stream_with_timing_mismatch_across_temporal_units() -> Vec<u8> {
    let other = CiTiming {
        time_scale: 60000,
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(temporal_delimiter_obu());
    data.extend(content_interpretation_obu(1, 0, Some(other)));
    data
}

#[test]
fn ci_timing_mismatch_across_temporal_units_without_clk_is_flagged() {
    let data = stream_with_timing_mismatch_across_temporal_units();
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-header/timing-time-scale-mismatch"),
        "a cross-temporal-unit timing mismatch without a CLK stays in the same \
         coded video sequence and must be flagged; report was: {report}"
    );
}

#[test]
fn ci_timing_mismatch_in_clk_temporal_unit_is_not_flagged() {
    let mut data = stream_with_timing_mismatch_across_temporal_units();
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("sequence-header/timing-")),
        "a CLK in the differing CI's temporal unit starts a new coded video \
         sequence that the CI joins; report was: {report}"
    );
}

#[test]
fn ci_repeated_for_same_embedded_layer_with_different_payload_is_flagged() {
    let other = CiTiming {
        time_scale: 24000,
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(content_interpretation_obu(0, 0, Some(other)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn ci_repeated_identical_for_same_embedded_layer_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn ci_repeated_non_identical_across_temporal_units_without_clk_is_flagged() {
    let other = CiTiming {
        time_scale: 24000,
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(temporal_delimiter_obu());
    data.extend(content_interpretation_obu(0, 0, Some(other)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "a cross-temporal-unit repeat without a CLK stays in the same coded \
         video sequence and must be flagged; report was: {report}"
    );
}

#[test]
fn ci_repeated_non_identical_in_clk_temporal_unit_is_not_flagged() {
    let other = CiTiming {
        time_scale: 24000,
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(temporal_delimiter_obu());
    data.extend(content_interpretation_obu(0, 0, Some(other)));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "a CLK in the repeat's temporal unit starts a new coded video sequence \
         that the repeat joins; report was: {report}"
    );
}

#[test]
fn ci_in_later_celu_absent_from_first_celu_of_sequence_is_flagged() {
    let mut data = temporal_delimiter_obu(); // TU0: the CVS's first temporal unit
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1))); // seq xlayer 0, mlayer 0,1
    data.extend(clk_frame_for_xlayer(0, 0)); // CLK frame -> starts the CVS, no CI in first CELU
    data.extend(temporal_delimiter_obu()); // TU1: continues the same CVS (no CLK)
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1))); // resend the header
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING))); // CI for mlayer 0
    data.extend(frame_obu_direct_seq_ref(0x1C, 0)); // a frame so the later CELU has an output
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/content-interpretation-not-in-first-celu"),
        "a later CELU carries a CI for an embedded layer the sequence's first CELU lacked, \
         so § 7.3.6 lines 560-562 must fire; report was: {report}"
    );
}

#[test]
fn ci_in_first_celu_of_sequence_then_repeated_later_is_silent() {
    let mut data = temporal_delimiter_obu(); // TU0: the CVS's first temporal unit
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING))); // CI in the first CELU
    data.extend(clk_frame_for_xlayer(0, 0)); // CLK frame -> starts the CVS
    data.extend(temporal_delimiter_obu()); // TU1: continues the same CVS
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING))); // identical repeat
    data.extend(frame_obu_direct_seq_ref(0x1C, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/content-interpretation-not-in-first-celu"),
        "the CI is present in the first CELU of the sequence, so the presence rule must \
         stay silent; report was: {report}"
    );
}

#[test]
fn ci_in_later_celu_with_mid_cvs_join_is_silent() {
    let mut data = temporal_delimiter_obu(); // TU0 (no CLK -> implicit CVS, first CELU unseen)
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(frame_obu_direct_seq_ref(0x1C, 0)); // a non-key frame, no CVS start
    data.extend(temporal_delimiter_obu()); // TU1 (still no CLK)
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING))); // CI in a later CELU
    data.extend(frame_obu_direct_seq_ref(0x1C, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/content-interpretation-not-in-first-celu"),
        "a mid-CVS join (no observed first CELU) must drop the presence judgment; report \
         was: {report}"
    );
}

#[test]
fn ci_first_celu_presence_judgment_resets_at_new_cvs() {
    let mut data = temporal_delimiter_obu(); // TU0: first CVS start
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING))); // CI in first CVS's first CELU
    data.extend(clk_frame_for_xlayer(0, 0)); // CLK -> first CVS
    data.extend(temporal_delimiter_obu()); // TU1: continues first CVS
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING))); // identical repeat (ok)
    data.extend(frame_obu_direct_seq_ref(0x1C, 0));
    data.extend(temporal_delimiter_obu()); // TU2: SECOND CVS start, no CI in its first CELU
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // CLK -> second CVS (resets the judgment)
    data.extend(temporal_delimiter_obu()); // TU3: continues second CVS, adds a CI in a later CELU
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING))); // CI absent from 2nd CVS's first CELU
    data.extend(frame_obu_direct_seq_ref(0x1C, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/content-interpretation-not-in-first-celu"),
        "the second coded video sequence's first CELU lacks the CI a later CELU adds, so \
         the per-CVS-reset presence rule must fire for it; report was: {report}"
    );
}

#[test]
fn ci_in_later_celu_under_external_hls_is_silent() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet};
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(clk_frame_for_xlayer(0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(frame_obu_direct_seq_ref(0x1C, 0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/content-interpretation-not-in-first-celu"),
        "an external-HLS Provided mode cannot enumerate an external first-CELU CI, so the \
         presence judgment must drop; report was: {report}"
    );
}

#[test]
fn ci_reserved_bits_nonzero_is_warned() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0b10, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .warnings()
            .any(|d| d.rule_id == "content-interpretation/reserved-bits-nonzero"),
        "report was: {report}"
    );
    assert!(
        conformant_apart_from_header_only_celu(&report),
        "report was: {report}"
    );
}

#[test]
fn ci_repeat_differing_only_in_reserved_bits_is_not_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(content_interpretation_obu(0, 0b11, Some(BASE_TIMING)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

/// Content-interpretation OBU (xlayer 0 / mlayer 0) carrying a chroma sample
/// position (interlace scan type, so top and bottom are coded independently).
pub(in crate::validator::tests) fn content_interpretation_chroma_obu(
    top: u32,
    bottom: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(2, 2); // ci_scan_type_idc = 2 (interlace) -> bottom coded
    bits.bit(0); // ci_color_description_present_flag
    bits.bit(1); // ci_chroma_sample_position_present_flag
    bits.bit(0); // ci_aspect_ratio_info_present_flag
    bits.bit(0); // ci_timing_info_present_flag
    bits.f(0, 2); // ci_reserved_2bit
    bits.uvlc(top);
    bits.uvlc(bottom);
    bits.bit(0); // obu_extension_flag
    bits.bit(1); // trailing_one_bit
    annex_b_obu_with_header(&layer_obu_header(24, 0, 0, 0), &bits.into_bytes())
}

/// Content-interpretation OBU (xlayer 0 / mlayer 0) carrying an aspect-ratio idc.
pub(in crate::validator::tests) fn content_interpretation_aspect_obu(idc: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 2); // ci_scan_type_idc
    bits.bit(0); // ci_color_description_present_flag
    bits.bit(0); // ci_chroma_sample_position_present_flag
    bits.bit(1); // ci_aspect_ratio_info_present_flag
    bits.bit(0); // ci_timing_info_present_flag
    bits.f(0, 2); // ci_reserved_2bit
    bits.f(idc, 8); // ci_aspect_ratio_idc (!= 255 -> no extended SAR)
    bits.bit(0); // obu_extension_flag
    bits.bit(1); // trailing_one_bit
    annex_b_obu_with_header(&layer_obu_header(24, 0, 0, 0), &bits.into_bytes())
}

#[test]
fn ci_chroma_sample_position_out_of_range_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_chroma_obu(6, 0)); // top = 6 > 5
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/chroma-sample-position-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn ci_chroma_sample_position_in_range_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_chroma_obu(5, 0)); // both <= 5
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/chroma-sample-position-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn ci_aspect_ratio_idc_out_of_range_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_aspect_obu(17)); // 16 < 17 < 255
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/aspect-ratio-idc-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn ci_aspect_ratio_idc_extended_marker_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_extended_sar_obu(16, 9));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/aspect-ratio-idc-out-of-range"),
        "report was: {report}"
    );
}

/// CI OBU (xlayer 0 / mlayer 0) carrying a color description with the given
/// `ci_color_description_idc` (idc < 4, so the `rg(2)` prefix is a single zero
/// bit). When idc == 0 an explicit BT.709 triple is coded.
pub(in crate::validator::tests) fn content_interpretation_color_obu(color_idc: u32) -> Vec<u8> {
    assert!(
        color_idc < 4,
        "content_interpretation_color_obu only encodes idc < 4; use content_interpretation_color_custom_obu"
    );
    content_interpretation_color_custom_obu(color_idc, None, false)
}

#[test]
fn ci_repeat_differing_only_in_color_encoding_is_not_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_color_obu(1)); // BT.709 preset
    data.extend(content_interpretation_color_obu(0)); // explicit BT.709 triple
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}
