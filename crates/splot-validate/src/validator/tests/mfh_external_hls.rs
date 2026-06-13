// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// Multi-frame header OBU (type 3) at xlayer 0 referencing `seq_header_id`.
pub(in crate::validator::tests) fn multi_frame_header_obu(seq_header_id: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(seq_header_id); // mfh_seq_header_id
    bits.uvlc(0); // mfh_id_minus_1 -> mfhId = 1
    bits.bit(0); // mfh_frame_size_present_flag
    bits.bit(0); // mfh_deblocking_filter_update
    bits.bit(0); // mfh_seg_info_present_flag -> fully parsed
    bits.bit(0); // obu_extension_flag = 0
    bits.bit(1); // trailing_one_bit
    annex_b_obu(0x0C, &bits.into_bytes())
}

/// Temporal delimiter + an activating sequence header with `seq_header_id` for
/// xlayer 0, then a multi-frame header referencing `mfh_seq_header_id`.
pub(in crate::validator::tests) fn stream_with_mfh_reference(
    seq_header_id: u32,
    mfh_seq_header_id: u32,
) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_id(seq_header_id, 1, 1),
    ));
    data.extend(multi_frame_header_obu(mfh_seq_header_id));
    data
}

#[test]
fn mfh_referencing_available_sequence_header_is_accepted() {
    let data = stream_with_mfh_reference(0, 0);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
        "report was: {report}"
    );
}

#[test]
fn mfh_referencing_missing_sequence_header_is_flagged() {
    // Only seq_header_id 0 is in-band; the MFH references 5. Default options do
    // not assume any external HLS, so this is unavailable.
    let data = stream_with_mfh_reference(0, 5);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
        "report was: {report}"
    );
}

#[test]
fn mfh_unavailable_under_default_options_emits_external_hls_disabled_advisory() {
    let data = stream_with_mfh_reference(0, 5);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .warnings()
            .any(|d| d.rule_id == "hls/external-hls-disabled"),
        "report was: {report}"
    );
}

#[test]
fn mfh_reference_satisfied_by_external_hls_is_accepted() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let data = stream_with_mfh_reference(0, 5);
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(5)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
        "report was: {report}"
    );
    assert!(
        !report
            .warnings()
            .any(|d| d.rule_id == "hls/external-hls-disabled"),
        "report was: {report}"
    );
}

#[test]
fn mfh_reference_not_in_external_hls_set_is_flagged_without_advisory() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // External HLS is Provided but does not declare id 5, so the reference is
    // genuinely unavailable: the error fires, but the external-hls-disabled
    // advisory must be suppressed (external HLS is not disabled).
    let data = stream_with_mfh_reference(0, 5);
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(99)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
        "report was: {report}"
    );
    assert!(
        !report
            .warnings()
            .any(|d| d.rule_id == "hls/external-hls-disabled"),
        "advisory must be suppressed when external HLS is Provided; report was: {report}"
    );
}

#[test]
fn external_hls_suppresses_no_active_sequence_header() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // A multi-frame header at xlayer 0 with no in-band sequence header at all.
    let mut data = temporal_delimiter_obu();
    data.extend(multi_frame_header_obu(5));

    // Default (external disabled): no in-band active sequence -> flagged.
    let default_report = Validator::new(false).validate_bytes(&data);
    assert!(
        default_report
            .errors()
            .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
        "report was: {default_report}"
    );

    // External HLS provided: an external sequence header may be the active one,
    // so the missing-in-band-sequence error is suppressed (the validator must not
    // reject a conformant external-HLS stream), and the referenced id 5 resolves
    // externally.
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(5)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
        "report was: {report}"
    );
}

#[test]
fn external_hls_empty_set_does_not_suppress_no_active_sequence_header() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // An empty external set declares no sequence header that could be active, so
    // the missing-active-header error must NOT be suppressed (otherwise an empty
    // set would silently accept a malformed stream).
    let mut data = temporal_delimiter_obu();
    data.extend(multi_frame_header_obu(5));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
        "report was: {report}"
    );
}

#[test]
fn external_hls_suppresses_active_sequence_layer_limits() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // In-band active sequence (id 0) for xlayer 0 allows only embedded layer 0,
    // then a coded OBU at embedded layer 1.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 1, 0), &[]));

    // Default: the OBU exceeds the in-band active sequence's mlayer limit.
    let default_report = Validator::new(false).validate_bytes(&data);
    assert!(
        default_report
            .errors()
            .any(|d| d.rule_id == "sequence-state/mlayer-exceeds-max"),
        "report was: {default_report}"
    );

    // External HLS declaring a sequence header: a different external sequence may
    // be active with limits this validator does not model, so the in-band
    // layer-limit check is suppressed (sound: never reject a conformant
    // external-HLS stream).
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/mlayer-exceeds-max"),
        "report was: {report}"
    );
}

#[test]
fn mfh_reference_to_malformed_tail_sequence_header_is_unavailable() {
    // A sequence header whose body parses but whose §5.2.1 payload tail is
    // malformed (an extra non-zero trailing byte) is not a valid available HLS
    // object, so it is not recorded — a later MFH referencing it is unavailable.
    let mut data = temporal_delimiter_obu();
    // A well-formed activating sequence header (id 0) so the MFH has an active
    // sequence for its xlayer.
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    // Sequence header id 7 with a malformed tail (trailing zero bit not zero).
    let mut malformed = sequence_header_payload_with_id(7, 0, 0);
    malformed.push(0xFF);
    data.extend(annex_b_obu(0x04, &malformed));
    // Multi-frame header referencing id 7.
    data.extend(multi_frame_header_obu(7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
        "report was: {report}"
    );
}

#[test]
fn mfh_reference_to_malformed_layer_sequence_header_is_unavailable() {
    // A sequence header with a §6.2.2 layer violation (tlayer != 0, 0x05) is
    // malformed and is NOT a valid available HLS object, so an MFH referencing
    // only that copy of id 4 is unavailable (§7.3.8.6).
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x05, &sequence_header_payload_with_id(4, 1, 1)));
    // An activating base-layer header (id 0) so the MFH has an active sequence for
    // its xlayer; it does not make id 4 available.
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(multi_frame_header_obu(4));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "mfh/sequence-header-unavailable"),
        "report was: {report}"
    );
}

#[test]
fn external_hls_out_of_range_id_does_not_suppress_no_active_sequence_header() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // Declaring an out-of-range external id (16 >= MAX_SEQ_NUM) cannot make a
    // valid sequence header available, so it must not suppress the missing-active
    // error.
    let mut data = temporal_delimiter_obu();
    data.extend(multi_frame_header_obu(5));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(16)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
        "report was: {report}"
    );
}

#[test]
fn ci_repeat_differing_in_color_preset_is_flagged() {
    // Genuinely different color information (BT.709 vs BT.2100 PQ) is a §6.14
    // violation and must be flagged even though both are preset encodings.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_color_obu(1)); // BT.709 SDR
    data.extend(content_interpretation_color_obu(2)); // BT.2100 PQ
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn ci_repeat_differing_in_aspect_preset_is_flagged() {
    // Different aspect ratios (1:1 vs 12:11) are different information (§6.14).
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_aspect_obu(1)); // SAR 1:1
    data.extend(content_interpretation_aspect_obu(2)); // SAR 12:11
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

/// CI OBU (xlayer 0 / mlayer 0) carrying an explicit sample aspect ratio
/// (`ci_aspect_ratio_idc == 255`).
pub(in crate::validator::tests) fn content_interpretation_extended_sar_obu(
    sar_width: u32,
    sar_height: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 2); // ci_scan_type_idc
    bits.bit(0); // ci_color_description_present_flag
    bits.bit(0); // ci_chroma_sample_position_present_flag
    bits.bit(1); // ci_aspect_ratio_info_present_flag
    bits.bit(0); // ci_timing_info_present_flag
    bits.f(0, 2); // ci_reserved_2bit
    bits.f(255, 8); // ci_aspect_ratio_idc = 255 -> extended SAR
    bits.uvlc(sar_width);
    bits.uvlc(sar_height);
    bits.bit(0); // obu_extension_flag
    bits.bit(1); // trailing_one_bit
    annex_b_obu_with_header(&layer_obu_header(24, 0, 0, 0), &bits.into_bytes())
}

#[test]
fn ci_repeat_alias_equivalent_aspect_is_not_flagged() {
    // Aspect preset idc 1 derives to SAR 1:1, the same as the explicit 255-coded
    // SAR 1:1, so the alias-equivalent re-encoding must not be flagged.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_aspect_obu(1)); // preset SAR 1:1
    data.extend(content_interpretation_extended_sar_obu(1, 1)); // explicit SAR 1:1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn ci_repeat_unreduced_explicit_sar_is_not_flagged() {
    // SAR 2:2 reduces to 1:1, the same ratio as the preset idc 1, so the
    // unreduced explicit re-encoding must not be flagged.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_aspect_obu(1)); // preset SAR 1:1
    data.extend(content_interpretation_extended_sar_obu(2, 2)); // explicit SAR 2:2 == 1:1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn ci_present_color_difference_after_absent_baseline_is_flagged() {
    // An absent-color first CI must not hide a genuine difference between two
    // later PRESENT color descriptions (absent -> BT.709 -> BT.2100). The
    // baseline records the first present color so the present-vs-present
    // difference is detected.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_obu(0, 0, None)); // color absent
    data.extend(content_interpretation_color_obu(1)); // BT.709
    data.extend(content_interpretation_color_obu(2)); // BT.2100 PQ
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn ci_present_aspect_difference_after_absent_baseline_is_flagged() {
    // Same as above for aspect ratio: absent -> SAR 1:1 -> SAR 12:11.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_obu(0, 0, None)); // aspect absent
    data.extend(content_interpretation_aspect_obu(1)); // SAR 1:1
    data.extend(content_interpretation_aspect_obu(2)); // SAR 12:11
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

/// CI OBU (xlayer 0 / mlayer 0) carrying a color description with an arbitrary
/// `ci_color_description_idc` (properly `rg(2)`-encoded), the explicit triple when
/// `idc == 0`, and the given full-range flag.
pub(in crate::validator::tests) fn content_interpretation_color_custom_obu(
    color_idc: u32,
    triple: Option<(u8, u8, u8)>,
    full_range: bool,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 2); // ci_scan_type_idc
    bits.bit(1); // ci_color_description_present_flag
    bits.bit(0); // ci_chroma_sample_position_present_flag
    bits.bit(0); // ci_aspect_ratio_info_present_flag
    bits.bit(0); // ci_timing_info_present_flag
    bits.f(0, 2); // ci_reserved_2bit
    // rg(2): (idc >> 2) one bits, a terminating zero, then the 2-bit remainder.
    for _ in 0..(color_idc >> 2) {
        bits.bit(1);
    }
    bits.bit(0);
    bits.f(color_idc & 0b11, 2);
    if color_idc == 0 {
        let (cp, tc, mc) = triple.unwrap_or((1, 1, 1));
        bits.f(u32::from(cp), 8);
        bits.f(u32::from(tc), 8);
        bits.f(u32::from(mc), 8);
    }
    bits.bit(u8::from(full_range)); // ci_full_range_flag
    bits.bit(0); // obu_extension_flag
    bits.bit(1); // trailing_one_bit
    annex_b_obu_with_header(&layer_obu_header(24, 0, 0, 0), &bits.into_bytes())
}

#[test]
fn ci_repeat_reserved_color_vs_explicit_unspecified_is_not_flagged() {
    // A reserved color id (6) is decoder-ignored -> unspecified (2, 2, 2), the
    // same derived color as an explicit (2, 2, 2) with the same full-range flag,
    // so the repeat must not be flagged.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_color_custom_obu(6, None, false)); // reserved
    data.extend(content_interpretation_color_custom_obu(
        0,
        Some((2, 2, 2)),
        false,
    )); // explicit unspecified
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn ci_repeat_present_color_vs_absent_default_is_flagged() {
    // An absent color description defaults to unspecified (2, 2, 2); a present
    // BT.709 carries different information, so the repeat is flagged.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_obu(0, 0, None)); // color absent
    data.extend(content_interpretation_color_obu(1)); // BT.709
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn ci_repeat_present_aspect_vs_absent_default_is_flagged() {
    // An absent aspect ratio defaults to unspecified (0, 0); a present SAR 1:1
    // carries different information, so the repeat is flagged.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_obu(0, 0, None)); // aspect absent
    data.extend(content_interpretation_aspect_obu(1)); // SAR 1:1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn ci_repeat_both_absent_color_and_aspect_is_not_flagged() {
    // Two CIs that both omit color and aspect carry the same (unspecified)
    // information and must not be flagged.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_obu(0, 0, None));
    data.extend(content_interpretation_obu(0, 0, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn ci_zero_display_tick_is_reported_under_timing_namespace() {
    // A timing-range violation carried by a content-interpretation OBU is
    // reported under the §6.4.12 timing namespace (sequence-header/timing-*).
    // §6.4.12 "Timing info semantics" is a subsection of §6.4 "Sequence header
    // OBU semantics", so the namespace follows the spec's section hierarchy and
    // is consistent with the cross-layer timing-mismatch diagnostics; the
    // diagnostic's spec_section (6.4.12) and byte offset locate it precisely.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 0)));
    data.extend(content_interpretation_obu(
        0,
        0,
        Some(CiTiming {
            display_tick: 0, // num_units_in_display_tick == 0 -> §6.4.12 violation
            time_scale: 30000,
            equal_picture_interval: false,
            num_ticks_minus_1: 0,
        }),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-header/timing-display-tick-zero"
                && d.spec_section.as_deref() == Some("6.4.12")
        }),
        "report was: {report}"
    );
}
