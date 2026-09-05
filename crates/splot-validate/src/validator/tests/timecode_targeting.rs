// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// A global `OBU_METADATA_GROUP` (xlayer 31) carrying one non-cancel timecode unit
/// (type 4) with `muh_layer_idc = LAYER_VALUES`, `muh_xlayer_map` selecting the
/// single extended layer `xlayer_bit`, and a single `muh_mlayer_map` byte targeting
/// the embedded layers in `mlayer_map`. No temporal-delimiter prefix (chainable).
pub(in crate::validator::tests) fn global_timecode_group_layer_values_obu(
    xlayer_bit: u8,
    mlayer_map: u8,
    unit: &[u8],
) -> Vec<u8> {
    assert!(unit.len() < 128, "timecode unit fits a 1-byte leb128");
    assert!(
        xlayer_bit < 31,
        "muh_xlayer_map bit 31 must be 0 (§ 6.16.3)"
    );
    let payload_size = unit.len() as u8;
    let xlayer_map = 1u32 << xlayer_bit;
    let xlayer_map_bytes = xlayer_map.to_be_bytes();
    let mut payload = vec![
        0x00, // is_suffix=0, necessity=0, application_id=0
        0x00, // metadata_unit_cnt_minus_1 = 0
        0x04, // metadata_type = 4 (METADATA_TYPE_TIMECODE)
        0x10, // muh_header_size = 8, cancel = 0
        payload_size,
        0x60,
        0x00, // layer_idc=LAYER_VALUES(3), persistence=0, priority=0, reserved=0
    ];
    payload.extend_from_slice(&xlayer_map_bytes); // muh_xlayer_map (4 bytes)
    payload.push(mlayer_map); // muh_mlayer_map for the selected extended layer
    payload.extend_from_slice(unit);
    payload.push(0x80); // OBU trailing byte
    annex_b_obu_with_header(&layer_obu_header(9, 0, 0, 31), &payload)
}

/// [`global_timecode_group_layer_values_obu`] for extended layer 0, with a
/// temporal-delimiter prefix.
pub(in crate::validator::tests) fn global_timecode_group_layer_values(
    mlayer_map: u8,
    unit: &[u8],
) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(global_timecode_group_layer_values_obu(0, mlayer_map, unit));
    data
}

#[test]
fn metadata_timecode_n_frames_targeting_excludes_untargeted_layer_ci() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(content_interpretation_obu(1, 0, Some(BASE_TIMING)));
    let unit = timecode_unit_bits(2, Some(0), Some(0), Some(0));
    data.extend(global_timecode_group_layer_values(0x02, &unit));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "a LAYER_VALUES timecode targeting layer 1 only must not pair with layer 0's \
         CI timing; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_targeting_pairs_with_targeted_layer_ci() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(content_interpretation_obu(1, 0, Some(low_rate)));
    let unit = timecode_unit_bits(2, Some(0), Some(0), Some(0));
    data.extend(global_timecode_group_layer_values(0x02, &unit));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "a LAYER_VALUES timecode targeting layer 1 must pair with layer 1's CI; \
         report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_olk_reinit_drops_deferred_pairing() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(temporal_delimiter_obu()); // -> TU1
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(open_loop_key_obu()); // OBU_OPEN_LOOP_KEY, xlayer 0 -> § 7.3.8.11 RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "the OLK reinitializes ci_timing_info_present_flag to 0, so the deferred \
         pairing against the prior-TU CI must drop; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_rap_resent_identical_ci_repairs() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    let timecode_offset = data.len() as u64 + 1;
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-n-frames-exceeds-rate"
                && d.byte_offset.map(splot_core::span::ByteOffset::get) == Some(timecode_offset)
        }),
        "the post-RAP re-sent CI must re-pair the RAP-temporal-unit timecode even \
         though its timing equals the pre-RAP copy's; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_rap_resent_identical_ci_before_timecode_reports_once() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        1,
        "an identical CI re-sent BEFORE the violating timecode in the RAP temporal unit \
         is paired-and-emitted eagerly; the CLK repair hook must not re-pair it and \
         duplicate the diagnostic; report was: {report}"
    );
}

/// Counts `metadata/timecode-n-frames-exceeds-rate` errors whose message names the
/// given `obu_xlayer_id` content interpretation, so a per-layer pairing can be
/// asserted independently of the other layer's.
pub(in crate::validator::tests) fn n_frames_errors_for_xlayer(
    report: &ValidationReport,
    xlayer: u8,
) -> usize {
    report
        .errors()
        .filter(|d| {
            d.rule_id == "metadata/timecode-n-frames-exceeds-rate"
                && d.message
                    .contains(&format!("obu_xlayer_id {xlayer} / obu_mlayer_id 0"))
        })
        .count()
}

#[test]
fn metadata_timecode_n_frames_rap_eager_pairing_for_one_layer_does_not_suppress_other() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu_at(0, 0, 0, Some(low_rate)));
    data.extend(content_interpretation_scan_obu_at(1, 0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    data.extend(content_interpretation_scan_obu_at(0, 0, 0, Some(low_rate)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(content_interpretation_scan_obu_at(1, 0, 0, Some(low_rate)));
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 1), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        n_frames_errors_for_xlayer(&report, 0),
        1,
        "extended layer 0's violation (paired eagerly before the timecode) must report \
         exactly once; report was: {report}"
    );
    assert_eq!(
        n_frames_errors_for_xlayer(&report, 1),
        1,
        "extended layer 1's violation (re-paired at the CLK after a deferred-and-dropped \
         pairing) must report exactly once — a per-observation eager flag would wrongly \
         suppress it; report was: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        2,
        "exactly one violation per layer, no duplicates; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_double_clk_same_layer_same_tu_repairs_once() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 0), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        1,
        "two CLKs for the same extended layer in one temporal unit must not run the RAP \
         re-pair twice and duplicate the diagnostic; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_olk_rap_resent_identical_ci_repairs() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    let timecode_offset = data.len() as u64 + 1;
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(open_loop_key_obu()); // OBU_OPEN_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-n-frames-exceeds-rate"
                && d.byte_offset.map(splot_core::span::ByteOffset::get) == Some(timecode_offset)
        }),
        "the post-OLK re-sent CI must re-pair the RAP-temporal-unit timecode even \
         though its timing equals the pre-RAP copy's; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_rap_resent_different_ci_reports_once() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let changed_rate = CiTiming {
        display_tick: 1000,
        time_scale: 3000, // maxPicPerSecond = ceil(3000 / 2000) = 2 (differs from low_rate)
        equal_picture_interval: true,
        num_ticks_minus_1: 1,
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(content_interpretation_obu(0, 0, Some(changed_rate)));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        1,
        "a post-RAP CI re-sent with CHANGED timing rechecks eagerly at CI-time; the \
         CLK repair hook must not re-pair it and duplicate the diagnostic; report \
         was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_identical_ci_repeat_no_rap_reports_once() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(5, Some(0), Some(0), Some(0)),
    ));
    data.extend(temporal_delimiter_obu()); // -> TU1, no CLK / OLK (no RAP)
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        1,
        "an identical CI repeat in a later temporal unit with no random access point \
         must not re-report the already-paired observation; report was: {report}"
    );
}

#[test]
fn scan_type_identical_ci_repeat_no_rap_reports_once() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(global_scan_type_obu(0x00, 0)); // Frame group, requires idc 1
    data.extend(temporal_delimiter_obu()); // -> TU1, no CLK / OLK (no RAP)
    data.extend(content_interpretation_scan_obu(2, None)); // identical re-send
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        1,
        "an identical CI repeat in a later temporal unit with no random access point \
         must not re-report the already-paired observation; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_global_layer_values_survives_other_layer_clk() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    let unit = timecode_unit_bits(5, Some(0), Some(0), Some(0));
    data.extend(global_timecode_group_layer_values_obu(1, 0x01, &unit));
    data.extend(temporal_delimiter_obu()); // -> TU1
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 0), &[]));
    data.extend(content_interpretation_scan_obu_at(1, 0, 0, Some(low_rate)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "a CLK for extended layer 0 must not prune a global timecode observation \
         targeting extended layer 1; report was: {report}"
    );
}

#[test]
fn metadata_timecode_inference_global_layer_values_pending_survives_other_layer_clk() {
    let build = |clk_xlayer: u8| {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        let seed = timecode_unit_bits(0, Some(0), Some(0), Some(0));
        data.extend(global_timecode_group_layer_values_obu(1, 0x01, &seed));
        data.extend(temporal_delimiter_obu()); // -> TU1
        let omitting = timecode_unit_bits(0, None, None, None);
        data.extend(global_timecode_group_layer_values_obu(1, 0x01, &omitting));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(4, 0, 0, clk_xlayer),
            &[],
        ));
        data
    };
    let report = Validator::new(false).validate_bytes(&build(0));
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/timecode-inferred-without-previous"),
        "a CLK for extended layer 0 must not fire the pending inference of a global \
         LAYER_VALUES timecode targeting extended layer 1; report was: {report}"
    );
    let report = Validator::new(false).validate_bytes(&build(1));
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-inferred-without-previous"
                && d.message.contains("seconds_value")
        }),
        "a CLK for the TARGETED extended layer 1 must fire the deferred inference \
         diagnostic; report was: {report}"
    );
}

#[test]
fn metadata_timecode_inference_chain_global_layer_values_survives_other_layer_clk() {
    let build = |clk_xlayer: u8| {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        let seed = timecode_unit_bits(0, Some(0), Some(0), Some(0));
        data.extend(global_timecode_group_layer_values_obu(1, 0x01, &seed));
        data.extend(temporal_delimiter_obu()); // -> TU1
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(4, 0, 0, clk_xlayer),
            &[],
        ));
        data.extend(temporal_delimiter_obu()); // -> TU2
        let omitting = timecode_unit_bits(0, None, None, None);
        data.extend(global_timecode_group_layer_values_obu(1, 0x01, &omitting));
        data
    };
    let report = Validator::new(false).validate_bytes(&build(0));
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/timecode-inferred-without-previous"),
        "a CLK for extended layer 0 must not prune a global LAYER_VALUES inference \
         chain targeting extended layer 1; report was: {report}"
    );
    let report = Validator::new(false).validate_bytes(&build(1));
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-inferred-without-previous"
                && d.message.contains("seconds_value")
        }),
        "a CLK for the TARGETED extended layer 1 must reset the inference chain so the \
         TU2 omission has no previous present value; report was: {report}"
    );
}

#[test]
fn metadata_timecode_inference_keyed_per_targeted_embedded_layer() {
    const LAYER_CURRENT_FIRST: u8 = 0x20;
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 0),
        &metadata_short_payload(LAYER_CURRENT_FIRST, 4, &{
            let mut bits = Bits::default();
            bits.f(0, 5); // counting_type
            bits.bit(1); // full_timestamp_flag
            bits.bit(0); // discontinuity_flag
            bits.bit(0); // cnt_dropped_flag
            bits.f(0, 9); // n_frames
            bits.f(0, 6); // seconds_value
            bits.f(0, 6); // minutes_value
            bits.f(0, 5); // hours_value
            bits.f(0, 5); // time_offset_length = 0
            bits.align();
            bits.into_bytes()
        }),
    ));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 1, 0),
        &metadata_short_payload(
            LAYER_CURRENT_FIRST,
            4,
            &timecode_unit_bits(0, None, None, None),
        ),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-inferred-without-previous"
                && d.message.contains("seconds_value")
        }),
        "a (xlayer 0, mlayer 0) timecode must not seed the inference of a (xlayer 0, \
         mlayer 1) timecode (per-targeted-layer keying); report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_unspecified_targeting_compares_nothing() {
    let low_rate = CiTiming {
        time_scale: 1000, // maxPicPerSecond = ceil(1000 / 2000) = 1
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(low_rate)));
    let mut bits = Bits::default();
    bits.f(0, 5); // counting_type
    bits.bit(1); // full_timestamp_flag
    bits.bit(0); // discontinuity_flag
    bits.bit(0); // cnt_dropped_flag
    bits.f(5, 9); // n_frames
    bits.f(0, 6); // seconds_value
    bits.f(0, 6); // minutes_value
    bits.f(0, 5); // hours_value
    bits.f(0, 5); // time_offset_length = 0
    bits.align();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &metadata_short_payload(0x00, 4, &bits.into_bytes()),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "a LAYER_UNSPECIFIED timecode does not specify its layers, so the n_frames \
         bound must compare nothing (no false positive against layer 0's CI); \
         report was: {report}"
    );
}

#[test]
fn metadata_timecode_counting_type_reserved_is_warned() {
    let mut bits = Bits::default();
    bits.f(7, 5); // counting_type = 7 (reserved)
    bits.bit(1); // full_timestamp_flag
    bits.bit(0); // discontinuity_flag
    bits.bit(0); // cnt_dropped_flag
    bits.f(0, 9); // n_frames
    bits.f(0, 6); // seconds_value
    bits.f(0, 6); // minutes_value
    bits.f(0, 5); // hours_value
    bits.f(0, 5); // time_offset_length = 0
    bits.align();
    let payload = metadata_short_payload(0x00, 4, &bits.into_bytes());
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        has_warning(&report, "metadata/timecode-counting-type-reserved"),
        "report was: {report}"
    );
    assert!(
        report.is_conformant(),
        "a reserved counting_type is decoder-ignored, not a violation; \
         report was: {report}"
    );
}

#[test]
fn metadata_timecode_counting_type_defined_is_silent() {
    let mut bits = Bits::default();
    bits.f(6, 5); // counting_type = 6 (defined)
    bits.bit(1); // full_timestamp_flag
    bits.bit(0); // discontinuity_flag
    bits.bit(0); // cnt_dropped_flag
    bits.f(0, 9); // n_frames
    bits.f(0, 6); // seconds_value
    bits.f(0, 6); // minutes_value
    bits.f(0, 5); // hours_value
    bits.f(0, 5); // time_offset_length = 0
    bits.align();
    let payload = metadata_short_payload(0x00, 4, &bits.into_bytes());
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        !has_warning(&report, "metadata/timecode-counting-type-reserved"),
        "counting_type 6 is defined; report was: {report}"
    );
}
