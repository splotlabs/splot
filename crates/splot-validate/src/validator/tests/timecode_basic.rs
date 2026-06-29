// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn metadata_timecode_seconds_out_of_range_is_flagged() {
    let mut bits = Bits::default();
    bits.f(0, 5); // counting_type
    bits.bit(1); // full_timestamp_flag
    bits.bit(0); // discontinuity_flag
    bits.bit(0); // cnt_dropped_flag
    bits.f(0, 9); // n_frames
    bits.f(60, 6); // seconds_value = 60 (> 59)
    bits.f(0, 6); // minutes_value
    bits.f(0, 5); // hours_value
    bits.f(0, 5); // time_offset_length = 0
    bits.align();
    let payload = metadata_short_payload(0x00, 4, &bits.into_bytes());
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        has_error(&report, "metadata/timecode-seconds-out-of-range"),
        "report was: {report}"
    );
}

/// The short-form `metadata_short_obu()` first byte for a non-cancel timecode unit
/// with `muh_layer_idc = LAYER_GLOBAL (1)`: `metadata_is_suffix 0`, `muh_layer_idc
/// 1` (`f(3)`), `muh_cancel_flag 0`, `muh_persistence_idc 0` -> `0b0_001_0_000`.
/// LAYER_GLOBAL on a global (`obu_xlayer_id == GLOBAL_XLAYER_ID`) OBU derives to
/// `HdrAssociation::Universal` ("The metadata applies to all layers", § 6.16.3), so
/// the n_frames bound pairs with every in-scope content interpretation — the
/// "global timecode describes every layer" intent these helpers model. (A
/// LAYER_UNSPECIFIED 0x00 first byte would leave the targeting unspecified, which
/// the n_frames bound now compares NOTHING against — finding 4.)
pub(in crate::validator::tests) const TIMECODE_SHORT_LAYER_GLOBAL: u8 = 0x10;

/// Builds a full-timestamp `metadata_timecode()` short OBU payload (LAYER_GLOBAL)
/// with the given seconds/minutes/hours values and no time offset.
pub(in crate::validator::tests) fn timecode_short_payload(
    seconds: u32,
    minutes: u32,
    hours: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 5); // counting_type
    bits.bit(1); // full_timestamp_flag
    bits.bit(0); // discontinuity_flag
    bits.bit(0); // cnt_dropped_flag
    bits.f(0, 9); // n_frames
    bits.f(seconds, 6); // seconds_value
    bits.f(minutes, 6); // minutes_value
    bits.f(hours, 5); // hours_value
    bits.f(0, 5); // time_offset_length = 0
    bits.align();
    metadata_short_payload(TIMECODE_SHORT_LAYER_GLOBAL, 4, &bits.into_bytes())
}

#[test]
fn metadata_timecode_minutes_out_of_range_is_flagged() {
    let payload = timecode_short_payload(0, 60, 0); // minutes_value = 60 (> 59)
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        has_error(&report, "metadata/timecode-minutes-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn metadata_timecode_hours_out_of_range_is_flagged() {
    let payload = timecode_short_payload(0, 0, 24); // hours_value = 24 (> 23)
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        has_error(&report, "metadata/timecode-hours-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn metadata_timecode_in_range_is_accepted() {
    let payload = timecode_short_payload(59, 59, 23);
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("metadata/timecode-")),
        "report was: {report}"
    );
}

/// Builds a `metadata_timecode()` short OBU payload with `full_timestamp_flag = 0`
/// and per-field presence flags. `seconds`/`minutes`/`hours` are `Some(value)` when
/// the field is signaled (its enclosing flag set), `None` when absent. `n_frames`
/// is configurable. The hierarchical flags (§ 5.17.7) require seconds present for
/// minutes, and minutes present for hours — the helper asserts that invariant so a
/// test cannot encode an impossible bitstream.
pub(in crate::validator::tests) fn timecode_flagged_payload(
    n_frames: u32,
    seconds: Option<u32>,
    minutes: Option<u32>,
    hours: Option<u32>,
) -> Vec<u8> {
    metadata_short_payload(
        TIMECODE_SHORT_LAYER_GLOBAL,
        4,
        &timecode_unit_bits(n_frames, seconds, minutes, hours),
    )
}

/// The raw `metadata_timecode()` syntax bytes (no metadata-unit wrapper) for a
/// `full_timestamp_flag = 0` set with per-field presence flags. See
/// [`timecode_flagged_payload`] for the field semantics.
pub(in crate::validator::tests) fn timecode_unit_bits(
    n_frames: u32,
    seconds: Option<u32>,
    minutes: Option<u32>,
    hours: Option<u32>,
) -> Vec<u8> {
    assert!(
        !(minutes.is_some() && seconds.is_none()),
        "minutes_value requires seconds_value present (§ 5.17.7)"
    );
    assert!(
        !(hours.is_some() && minutes.is_none()),
        "hours_value requires minutes_value present (§ 5.17.7)"
    );
    let mut bits = Bits::default();
    bits.f(0, 5); // counting_type
    bits.bit(0); // full_timestamp_flag = 0 -> per-field flags
    bits.bit(0); // discontinuity_flag
    bits.bit(0); // cnt_dropped_flag
    bits.f(n_frames, 9); // n_frames
    bits.bit(u8::from(seconds.is_some())); // seconds_flag
    if let Some(s) = seconds {
        bits.f(s, 6); // seconds_value
        bits.bit(u8::from(minutes.is_some())); // minutes_flag
        if let Some(m) = minutes {
            bits.f(m, 6); // minutes_value
            bits.bit(u8::from(hours.is_some())); // hours_flag
            if let Some(h) = hours {
                bits.f(h, 5); // hours_value
            }
        }
    }
    bits.f(0, 5); // time_offset_length = 0
    bits.align();
    bits.into_bytes()
}

#[test]
fn metadata_timecode_inferred_seconds_without_previous_is_flagged() {
    let payload = timecode_flagged_payload(0, None, None, None);
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-inferred-without-previous"
                && d.message.contains("seconds_value")
        }),
        "report was: {report}"
    );
}

/// A global `OBU_METADATA_SHORT` (xlayer 31) carrying `payload`, with no temporal
/// delimiter prefix (for chaining several into one stream).
pub(in crate::validator::tests) fn global_metadata_short_obu(payload: &[u8]) -> Vec<u8> {
    annex_b_obu_with_header(&layer_obu_header(8, 0, 0, 31), payload)
}

#[test]
fn metadata_timecode_inference_after_present_value_passes() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_metadata_short_obu(&timecode_short_payload(
        12, 34, 5,
    )));
    data.extend(global_metadata_short_obu(&timecode_flagged_payload(
        0, None, None, None,
    )));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/timecode-inferred-without-previous"),
        "an omitted field after a present previous value infers cleanly; report was: {report}"
    );
}

#[test]
fn metadata_timecode_full_timestamp_first_passes() {
    let payload = timecode_short_payload(0, 0, 0);
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/timecode-inferred-without-previous"),
        "report was: {report}"
    );
}

#[test]
fn metadata_timecode_inference_names_each_absent_field() {
    let payload = timecode_flagged_payload(0, Some(30), None, None);
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    let inferred: Vec<&str> = report
        .errors()
        .filter(|d| d.rule_id == "metadata/timecode-inferred-without-previous")
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(inferred.len(), 2, "report was: {report}");
    assert!(
        inferred.iter().any(|m| m.contains("minutes_value"))
            && inferred.iter().any(|m| m.contains("hours_value"))
            && !inferred.iter().any(|m| m.contains("seconds_value")),
        "report was: {report}"
    );
}

#[test]
fn metadata_timecode_inference_chain_resets_at_clk() {
    let mut data = global_metadata_short_stream(&timecode_short_payload(0, 0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(0, None, None, None),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "metadata/timecode-inferred-without-previous"),
        "the inference chain must reset across the CLK boundary; report was: {report}"
    );
}

/// `BASE_TIMING` is display_tick 1000, time_scale 30000, equal_picture_interval
/// true, num_ticks_minus_1 1: TicksPerPicture = (1 + 1) * 1000 = 2000, so
/// maxPicPerSecond = ceil(30000 / 2000) = 15. n_frames must be < 15.

#[test]
fn metadata_timecode_n_frames_exceeds_rate_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(15, Some(0), Some(0), Some(0)),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_boundary_passes() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(14, Some(0), Some(0), Some(0)),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "n_frames == maxPicPerSecond - 1 must pass; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_without_timing_is_silent() {
    let payload = timecode_flagged_payload(400, Some(0), Some(0), Some(0));
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "absent CI timing means no n_frames bound; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_ci_arrives_after_metadata_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(15, Some(0), Some(0), Some(0)),
    ));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/timecode-n-frames-exceeds-rate"),
        1,
        "the bound is reported once, not per repeated CI; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_unequal_interval_bound() {
    let unequal = CiTiming {
        equal_picture_interval: false,
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(unequal)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(30, Some(0), Some(0), Some(0)),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "report was: {report}"
    );

    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(unequal)));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(29, Some(0), Some(0), Some(0)),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/timecode-n-frames-exceeds-rate"),
        "n_frames 29 == maxPicPerSecond - 1 must pass; report was: {report}"
    );
}

#[test]
fn metadata_timecode_omitted_after_omitted_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_metadata_short_obu(&timecode_short_payload(
        12, 34, 5,
    )));
    data.extend(global_metadata_short_obu(&timecode_flagged_payload(
        0, None, None, None,
    )));
    data.extend(global_metadata_short_obu(&timecode_flagged_payload(
        0, None, None, None,
    )));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-inferred-without-previous"
                && d.message.contains("seconds_value")
        }),
        "an omitted field whose predecessor only INFERRED it (never coded it) fires \
         under the literal reading; report was: {report}"
    );
}

#[test]
fn metadata_timecode_omitted_then_clk_in_same_tu_seeds_from_new_cvs() {
    let mut data = global_metadata_short_stream(&timecode_short_payload(0, 0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(global_metadata_short_obu(&timecode_flagged_payload(
        0, None, None, None,
    )));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-inferred-without-previous"
                && d.message.contains("seconds_value")
        }),
        "a same-TU CLK after the omitting timecode pulls it into the new CVS, so the \
         prior-CVS seed must not satisfy the inference; report was: {report}"
    );
}

#[test]
fn metadata_timecode_n_frames_ci_after_metadata_anchors_at_metadata() {
    let mut data = temporal_delimiter_obu();
    let timecode_offset = data.len() as u64 + 1;
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &timecode_flagged_payload(15, Some(0), Some(0), Some(0)),
    ));
    let ci_offset = data.len() as u64 + 1;
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "metadata/timecode-n-frames-exceeds-rate"
                && d.byte_offset.map(splot_core::span::ByteOffset::get) == Some(timecode_offset)
        }),
        "the diagnostic must anchor at the timecode metadata OBU (byte \
         {timecode_offset}), not the CI OBU (byte {ci_offset}); report was: {report}"
    );
}
