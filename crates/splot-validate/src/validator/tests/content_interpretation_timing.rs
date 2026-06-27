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
    // The § 6.4.12 cross-embedded-layer timing comparison is tagged with the
    // BASELINE record's temporal unit: with no CLK, temporal unit 2 continues
    // xlayer 0's coded video sequence (AV2 § 7.3.6), so the deferred
    // comparison must be emitted by the end-of-stream flush.
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
    // Same stream plus a CLK for xlayer 0 in temporal unit 2: per AV2 § 7.3.6
    // the new coded video sequence starts at the temporal unit, so the
    // embedded-layer-1 timing belongs to the NEW coded video sequence and the
    // deferred comparison against the old sequence's baseline is dropped (no
    // false positive at the exact CVS boundary).
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
    // Temporal unit 2 has no CLK, so per AV2 § 7.3.6 it continues xlayer 0's
    // coded video sequence from temporal unit 1: a repeated CI OBU for the same
    // embedded layer with different information is a § 6.14 violation. The
    // comparison is deferred (a CLK later in temporal unit 2 could still have
    // started a new coded video sequence) and emitted by the end-of-stream
    // flush.
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
    // Same stream, but temporal unit 2 contains a CLK for xlayer 0 after the
    // repeated CI OBU: per AV2 § 7.3.6 the new coded video sequence starts at
    // the temporal unit, so the differing CI joins the NEW coded video sequence
    // and the deferred cross-temporal-unit comparison is dropped (no false
    // positive).
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

// --- Round-6 F3: § 7.3.6 first-CELU-of-the-sequence CI PRESENCE rule (lines 560-562) ----

#[test]
fn ci_in_later_celu_absent_from_first_celu_of_sequence_is_flagged() {
    // Round-6 F3: § 7.3.6 (mirror lines 560-562) requires a CI present in any coded
    // extended layer unit to ALSO be present in the FIRST coded extended layer unit of the
    // sequence. TU0 starts the coded video sequence for xlayer 0 with a CLK frame and NO
    // content interpretation (the first CELU of the sequence lacks a CI for mlayer 0). TU1
    // (same CVS — no CLK) carries a CI for mlayer 0 in a LATER CELU. The presence half is
    // violated and must fire `celu/content-interpretation-not-in-first-celu`. The
    // contents-identity half (§ 6.14) is owned by `repeated-ci-not-identical` and is not
    // exercised here (no earlier copy to compare against).
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
    // Round-6 F3 control: the CI IS present in the first CELU of the sequence (TU0), then
    // repeated identically in a later CELU (TU1). The presence half is satisfied, so
    // `celu/content-interpretation-not-in-first-celu` must stay SILENT (and the identical
    // repeat does not trip `repeated-ci-not-identical` either).
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
    // Round-6 F3 drop: the stream starts MID-CVS — no CLK for xlayer 0 is ever observed, so
    // the first coded extended layer unit of the sequence was not observed. The presence
    // judgment is undecidable (the first CELU's CI set is unknowable), so a CI in a later
    // CELU must NOT fire `celu/content-interpretation-not-in-first-celu`.
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
    // Round-6 F3: a CLK starting a NEW coded video sequence resets the first-CELU CI
    // presence state. The first CVS (TU0–TU1) is CONFORMING: its first CELU (TU0) carries
    // the CI, repeated in TU1 — no fire. The second CVS begins at TU2's CLK whose first
    // CELU carries NO CI; TU3 (same second CVS) then adds a CI for mlayer 0 in a later CELU
    // -> the presence rule fires for the SECOND CVS only. This proves the judgment is reset
    // per CVS (the first CVS's first-CELU CI does not excuse the second CVS).
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
    // Round-6 F3 drop: under an external-HLS Provided mode an external CI in the first CELU
    // cannot be enumerated by ExternalHlsSet (it expresses only sequence headers and
    // operating point sets), so the presence judgment drops. The same stream as
    // `…_absent_from_first_celu_of_sequence_is_flagged` validated with a Provided set must
    // NOT fire `celu/content-interpretation-not-in-first-celu`.
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
    // A reserved-bits anomaly is a warning, not a conformance error. (The minimal
    // CI-only fixture is a header-only CELU, so set aside the expected
    // celu/missing-output-frame-unit.)
    assert!(
        conformant_apart_from_header_only_celu(&report),
        "report was: {report}"
    );
}

#[test]
fn ci_repeat_differing_only_in_reserved_bits_is_not_flagged() {
    // AV2 § 6.14: ci_reserved_2bit is decoder-ignored, so two CI OBUs for the
    // same embedded layer that differ only in the reserved bits carry the same
    // information and must not be flagged as a non-identical repeat.
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
    // ci_aspect_ratio_idc == 255 is the extended-SAR marker, not out of range.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    let mut bits = Bits::default();
    bits.f(0, 2); // ci_scan_type_idc
    bits.bit(0); // color description absent
    bits.bit(0); // chroma sample position absent
    bits.bit(1); // ci_aspect_ratio_info_present_flag
    bits.bit(0); // timing absent
    bits.f(0, 2); // ci_reserved_2bit
    bits.f(255, 8); // ci_aspect_ratio_idc = 255 -> extended SAR
    bits.uvlc(16); // ci_sar_width
    bits.uvlc(9); // ci_sar_height
    bits.bit(0); // obu_extension_flag
    bits.bit(1); // trailing_one_bit
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(24, 0, 0, 0),
        &bits.into_bytes(),
    ));
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
    // This helper encodes rg(2) with a single terminating zero bit (q == 0), so
    // it is only correct for idc < 4. Use content_interpretation_color_custom_obu
    // for larger ids (it emits the full rg(2) unary prefix).
    assert!(
        color_idc < 4,
        "content_interpretation_color_obu only encodes idc < 4; use content_interpretation_color_custom_obu"
    );
    let mut bits = Bits::default();
    bits.f(0, 2); // ci_scan_type_idc
    bits.bit(1); // ci_color_description_present_flag
    bits.bit(0); // ci_chroma_sample_position_present_flag
    bits.bit(0); // ci_aspect_ratio_info_present_flag
    bits.bit(0); // ci_timing_info_present_flag
    bits.f(0, 2); // ci_reserved_2bit
    bits.bit(0); // rg(2): q = 0 (terminating zero bit)
    bits.f(color_idc, 2); // rg(2): 2-bit remainder == idc for idc < 4
    if color_idc == 0 {
        bits.f(1, 8); // ci_color_primaries (BT.709)
        bits.f(1, 8); // ci_transfer_characteristics
        bits.f(1, 8); // ci_matrix_coefficients
    }
    bits.bit(0); // ci_full_range_flag
    bits.bit(0); // obu_extension_flag
    bits.bit(1); // trailing_one_bit
    annex_b_obu_with_header(&layer_obu_header(24, 0, 0, 0), &bits.into_bytes())
}

#[test]
fn ci_repeat_differing_only_in_color_encoding_is_not_flagged() {
    // AV2 § 6.14: color descriptions can encode the same information in multiple
    // ways (a Table 6.13 preset idc vs. the equivalent explicit triple). The
    // repeated-CI check compares *derived* values, so an alias-equivalent
    // re-encoding is not flagged (it must never false-positive a conformant
    // stream): BT.709 as preset idc 1 and as explicit (1, 1, 1) carry the same
    // information.
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
