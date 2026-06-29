// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// One `metadata_scan_type()` unit byte (AV2 § 5.17.10): `mps_pic_struct_type`
/// `f(5)`, `mps_source_scan_type_idc` `f(2)` (0 here — no consistency rule
/// binds it, § 6.16.10), `mps_duplicate_flag` `f(1)` (0).
pub(in crate::validator::tests) fn scan_type_unit(pic_struct: u8) -> [u8; 1] {
    [pic_struct << 3]
}

/// A global (xlayer 31) short metadata OBU carrying a scan-type unit (type 8);
/// `first` selects prefix (`0x00`) or suffix (`0x80`) placement and carries
/// `muh_layer_idc` / `muh_persistence_idc` 0.
pub(in crate::validator::tests) fn global_scan_type_obu(first: u8, pic_struct: u8) -> Vec<u8> {
    annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &metadata_short_payload(first, 8, &scan_type_unit(pic_struct)),
    )
}

/// An `OBU_CONTENT_INTERPRETATION` at the given layer ids carrying the given
/// `ci_scan_type_idc` and optional timing (all other optional branches
/// cleared), plus the § 5.2.1 extensible payload tail.
pub(in crate::validator::tests) fn content_interpretation_scan_obu_at(
    xlayer: u8,
    mlayer: u8,
    scan_type_idc: u32,
    timing: Option<CiTiming>,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(scan_type_idc, 2); // ci_scan_type_idc
    bits.bit(0); // ci_color_description_present_flag
    bits.bit(0); // ci_chroma_sample_position_present_flag
    bits.bit(0); // ci_aspect_ratio_info_present_flag
    bits.bit(u8::from(timing.is_some())); // ci_timing_info_present_flag
    bits.f(0, 2); // ci_reserved_2bit
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
    annex_b_obu_with_header(&layer_obu_header(24, 0, mlayer, xlayer), &bits.into_bytes())
}

/// [`content_interpretation_scan_obu_at`] at obu_xlayer_id 0 / obu_mlayer_id 0.
pub(in crate::validator::tests) fn content_interpretation_scan_obu(
    scan_type_idc: u32,
    timing: Option<CiTiming>,
) -> Vec<u8> {
    content_interpretation_scan_obu_at(0, 0, scan_type_idc, timing)
}

#[test]
fn scan_type_group_mixing_in_cvs_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 0));
    data.extend(global_scan_type_obu(0x00, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
        "report was: {report}"
    );
}

#[test]
fn scan_type_group_mixing_across_tu_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(global_scan_type_obu(0x00, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
        "a cross-temporal-unit group mix without a CLK stays in the same coded \
         video sequence and must be flagged; report was: {report}"
    );
}

#[test]
fn scan_type_group_change_after_clk_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(global_scan_type_obu(0x00, 3));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
        "a CLK in the temporal unit starts a new coded video sequence that the \
         group change joins; report was: {report}"
    );
}

#[test]
fn scan_type_group_mixing_between_global_and_xlayer_scopes_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 0));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(metadata_short_obu_at(0, 0x00, 8, &scan_type_unit(3)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
        "report was: {report}"
    );
}

#[test]
fn scan_type_group_mixing_between_xlayer_and_global_scopes_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(metadata_short_obu_at(0, 0x00, 8, &scan_type_unit(0)));
    data.extend(global_scan_type_obu(0x80, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
        "a global unit must be compared against an existing concrete \
         extended-layer baseline; report was: {report}"
    );
}

#[test]
fn scan_type_reserved_value_excluded_from_group_state() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 13));
    data.extend(global_scan_type_obu(0x00, 0));
    data.extend(global_scan_type_obu(0x00, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-pic-struct-reserved"),
        "report was: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "metadata/scan-type-pic-struct-group-inconsistent"),
        1,
        "only 0 vs 3 may conflict (13 is excluded); report was: {report}"
    );
}

#[test]
fn scan_type_ci_mismatch_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(1, None));
    data.extend(global_scan_type_obu(0x80, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        "report was: {report}"
    );

    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(1, None));
    data.extend(global_scan_type_obu(0x80, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn scan_type_ci_arrives_after_metadata_mismatch_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 3));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(1, None));
    data.extend(content_interpretation_scan_obu(1, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        1,
        "the mismatch is reported once, not per repeated CI; report was: {report}"
    );
}

#[test]
fn scan_type_frame_doubling_requires_equal_picture_interval() {
    let unequal = CiTiming {
        equal_picture_interval: false,
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(1, Some(unequal)));
    data.extend(global_scan_type_obu(0x80, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(
            &report,
            "metadata/scan-type-equal-picture-interval-required"
        ),
        "report was: {report}"
    );

    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(1, Some(BASE_TIMING)));
    data.extend(global_scan_type_obu(0x80, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(
            &report,
            "metadata/scan-type-equal-picture-interval-required"
        ),
        "report was: {report}"
    );

    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(1, None));
    data.extend(global_scan_type_obu(0x80, 7));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(
            &report,
            "metadata/scan-type-equal-picture-interval-required"
        ),
        "absent timing_info must stay silent; report was: {report}"
    );
}

#[test]
fn scan_type_without_ci_warns_unestablished_at_eos() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_warning(&report, "metadata/scan-type-ci-scan-type-unestablished"),
        "report was: {report}"
    );
    assert!(
        report.is_conformant(),
        "the unestablished case is a warning, not an error: {report}"
    );

    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(1, None));
    data.extend(global_scan_type_obu(0x80, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_warning(&report, "metadata/scan-type-ci-scan-type-unestablished"),
        "report was: {report}"
    );
}

#[test]
fn scan_type_unestablished_warned_at_cvs_restart() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .warnings()
            .filter(|d| d.rule_id == "metadata/scan-type-ci-scan-type-unestablished")
            .count(),
        1,
        "report was: {report}"
    );
}

#[test]
fn scan_type_ci_for_second_embedded_layer_rechecked() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 0)); // requires ci_scan_type_idc 1
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu_at(0, 0, 1, None)); // match
    data.extend(content_interpretation_scan_obu_at(0, 1, 2, None)); // mismatch
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        1,
        "the mlayer-1 content interpretation must be paired with the stored \
         observation; report was: {report}"
    );
    assert!(
        !has_error(&report, "content-interpretation/repeated-ci-not-identical"),
        "different embedded layers are distinct CI records (§ 6.14); \
         report was: {report}"
    );
}

#[test]
fn scan_type_ci_mismatch_on_second_xlayer_rechecked() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 0)); // requires ci_scan_type_idc 1
    data.extend(sequence_header_obu_for_xlayer(0, 0, 1));
    data.extend(content_interpretation_scan_obu_at(0, 0, 1, None)); // match
    data.extend(sequence_header_obu_for_xlayer(1, 0, 1));
    data.extend(content_interpretation_scan_obu_at(1, 0, 2, None)); // mismatch
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        "the xlayer-1 content interpretation must be paired with the global \
         observation; report was: {report}"
    );
}

#[test]
fn scan_type_equal_picture_interval_rechecked_for_second_layer() {
    let unequal = CiTiming {
        equal_picture_interval: false,
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 7));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu_at(0, 0, 1, None));
    data.extend(content_interpretation_scan_obu_at(0, 1, 1, Some(unequal)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(
            &report,
            "metadata/scan-type-equal-picture-interval-required"
        ),
        "report was: {report}"
    );
}

#[test]
fn scan_type_contradicting_ci_repeat_rechecked_and_co_reported() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_scan_type_obu(0x00, 0)); // requires ci_scan_type_idc 1
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu_at(0, 0, 1, None)); // match
    data.extend(content_interpretation_scan_obu_at(0, 0, 2, None)); // contradiction
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "content-interpretation/repeated-ci-not-identical"),
        "report was: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        1,
        "the changed decisive content must be re-paired exactly once; \
         report was: {report}"
    );
}

/// An `OBU_OPEN_LOOP_KEY` for xlayer 0 with an empty payload (the raw
/// OBU-header event is all the § 7.3.8.11 epoch tracking consumes).
pub(in crate::validator::tests) fn open_loop_key_obu() -> Vec<u8> {
    annex_b_obu(0x14, &[])
}

#[test]
fn scan_type_pre_olk_ci_not_paired_with_post_olk_metadata() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(temporal_delimiter_obu());
    data.extend(open_loop_key_obu());
    data.extend(temporal_delimiter_obu());
    data.extend(global_scan_type_obu(0x00, 0)); // requires ci_scan_type_idc 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        "the pre-OLK content interpretation no longer establishes the \
         parameters (§ 7.3.8.11); report was: {report}"
    );
}

#[test]
fn scan_type_ci_resent_at_olk_pairs_with_post_olk_metadata() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(temporal_delimiter_obu());
    data.extend(open_loop_key_obu());
    data.extend(content_interpretation_scan_obu(2, None)); // re-sent at the OLK
    data.extend(global_scan_type_obu(0x80, 0)); // suffix; requires idc 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        "a CI re-sent at the OLK re-establishes ci_scan_type_idc 2 for the \
         new epoch; report was: {report}"
    );
    assert!(
        !has_error(&report, "content-interpretation/repeated-ci-not-identical"),
        "the identical re-send is a legal § 6.14 repeat; report was: {report}"
    );
}

#[test]
fn scan_type_pre_olk_metadata_not_paired_with_olk_tu_ci() {
    for ci_before_olk in [true, false] {
        let mut data = temporal_delimiter_obu();
        data.extend(global_scan_type_obu(0x00, 0)); // requires idc 1
        data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
        data.extend(temporal_delimiter_obu());
        if ci_before_olk {
            data.extend(content_interpretation_scan_obu(2, None));
            data.extend(open_loop_key_obu());
        } else {
            data.extend(open_loop_key_obu());
            data.extend(content_interpretation_scan_obu(2, None));
        }
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
            "ci_before_olk={ci_before_olk}: the observation predates the \
             OLK's § 7.3.8.11 epoch; report was: {report}"
        );
    }
}

#[test]
fn scan_type_rap_resent_identical_ci_repairs() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    data.extend(global_scan_type_obu(0x00, 0));
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        "the post-RAP re-sent CI must re-pair the RAP-temporal-unit scan-type \
         metadata even though its ci_scan_type_idc equals the pre-RAP copy's; \
         report was: {report}"
    );
    assert!(
        !has_error(&report, "content-interpretation/repeated-ci-not-identical"),
        "the identical re-send is a legal § 6.14 repeat; report was: {report}"
    );
}

#[test]
fn scan_type_rap_resent_identical_ci_before_metadata_reports_once() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(global_scan_type_obu(0x00, 0));
    data.extend(annex_b_obu(0x10, &[])); // OBU_CLOSED_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        1,
        "an identical CI re-sent BEFORE the violating scan-type metadata in the RAP \
         temporal unit is paired-and-emitted eagerly; the CLK repair hook must not \
         re-pair it and duplicate the diagnostic; report was: {report}"
    );
}

#[test]
fn scan_type_olk_rap_resent_identical_ci_repairs() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    data.extend(global_scan_type_obu(0x00, 0));
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(open_loop_key_obu()); // OBU_OPEN_LOOP_KEY, xlayer 0 -> RAP
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        "the post-OLK re-sent CI must re-pair the RAP-temporal-unit scan-type \
         metadata even though its ci_scan_type_idc equals the pre-RAP copy's; \
         report was: {report}"
    );
    assert!(
        !has_error(&report, "content-interpretation/repeated-ci-not-identical"),
        "the identical re-send is a legal § 6.14 repeat; report was: {report}"
    );
}

#[test]
fn repeated_ci_differs_across_olk_still_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(1, None));
    data.extend(temporal_delimiter_obu());
    data.extend(open_loop_key_obu());
    data.extend(content_interpretation_scan_obu(2, None));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "content-interpretation/repeated-ci-not-identical"),
        "the § 6.14 identity rule is CVS-scoped, not RAP-scoped; \
         report was: {report}"
    );
}

#[test]
fn ci_timing_mismatch_across_olk_still_flagged() {
    let other = CiTiming {
        time_scale: 60000,
        ..BASE_TIMING
    };
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_obu(0, 0, Some(BASE_TIMING)));
    data.extend(temporal_delimiter_obu());
    data.extend(open_loop_key_obu());
    data.extend(content_interpretation_obu(1, 0, Some(other)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "sequence-header/timing-time-scale-mismatch"),
        "the § 6.4.12 timing rule is CVS-scoped, not RAP-scoped; \
         report was: {report}"
    );
}

#[test]
fn metadata_prefix_global_after_coded_layer_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[])); // coded layer
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &[0x08, 0x04, 0x80],
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "obu-order/global-hls-after-coded-layer"),
        "report was: {report}"
    );
}

#[test]
fn metadata_suffix_global_after_coded_layer_is_not_treated_as_prefix() {
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[])); // coded layer
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        &[0x88, 0x04, 0x80],
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "obu-order/global-hls-after-coded-layer"),
        "global suffix metadata must not be treated as a prefix; report was: {report}"
    );
}

#[test]
fn metadata_non_global_order_uses_coded_xlayer_order() {
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
    data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 0),
        &[0x08, 0x04, 0x80],
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "obu-order/xlayer-order-not-ascending"),
        "report was: {report}"
    );
}
