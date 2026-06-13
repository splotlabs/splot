// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

// --- scan-type CVS consistency (AV2 § 6.16.10 Table 6.18) ---

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
    // AV2 § 6.16.10: "only one of the following conditions, for all pictures in
    // the current CVS, is true" — mps_pic_struct_type 0 (group {0, 7 or 8}) and
    // 3 (group {3, 4, 5 or 6}) in the same temporal unit mix two Table 6.18
    // groups within one coded video sequence (emitted eagerly: same temporal
    // unit means same coded video sequence, § 7.3.6).
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
    // Temporal unit 2 has no CLK, so per AV2 § 7.3.6 it continues the coded
    // video sequence: the cross-temporal-unit group mix is deferred and emitted
    // by the end-of-stream flush.
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
    // Same stream, but temporal unit 2 contains a CLK: per AV2 § 7.3.6 the new
    // coded video sequence starts at the temporal unit, so mps_pic_struct_type
    // 3 belongs to the NEW coded video sequence and the deferred comparison
    // against the old sequence's group is dropped (no false positive).
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
    // Global scan-type metadata describes every layer's pictures, so a concrete
    // extended layer's group is checked against the global bucket's group.
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
    // Mirror of the pairing above: the concrete xlayer-0 scope establishes its
    // group baseline FIRST, and a later global-bucket unit of a different
    // Table 6.18 group must still be compared against that concrete scope
    // ("and vice versa") — the global unit is a suffix so it may follow the
    // coded-layer metadata OBU (§ 7.3.7).
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
    // "Decoders shall ignore reserved values of mps_pic_struct_type"
    // (AV2 § 6.16.10): the reserved value 13 gets only its own stateless
    // diagnostic and never enters the group state, so the group baseline is 0
    // and exactly one group error fires for 0 vs 3.
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
    // Table 6.18: mps_pic_struct_type 3 requires "ci_scan_type_idc shall be
    // equal to 3", but the in-scope content interpretation establishes 1
    // (progressive). The scan-type metadata is a global suffix unit so it may
    // follow the coded-layer content interpretation OBU (§ 7.3.7).
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(content_interpretation_scan_obu(1, None));
    data.extend(global_scan_type_obu(0x80, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/scan-type-ci-scan-type-mismatch"),
        "report was: {report}"
    );

    // Accepted twin: mps_pic_struct_type 0 requires ci_scan_type_idc 1, which
    // matches the established value.
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
    // Re-evaluation path: the scan-type metadata precedes the content
    // interpretation that decides its Table 6.18 restriction. A second
    // identical CI repeat must not re-report: its Table 6.18-decisive
    // content is unchanged, so the re-evaluation is skipped (§ 6.14 allows
    // exactly the identical repeat).
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
    // Table 6.18 for mps_pic_struct_type 7 (frame doubling): "ci_scan_type_idc
    // shall be equal to 1 and equal_picture_interval shall be equal to 1".
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

    // Accepted with equal_picture_interval == 1.
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

    // Silent when timing_info() is absent: the mirror attaches the restriction
    // to the signaled element and states no absent-timing rule (documented).
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
    // Derived literal reading of Table 6.18 (AV2 § 6.16.10): every defined
    // mps_pic_struct_type restricts ci_scan_type_idc to 1, 2 or 3, while the
    // § 7.3.8.11 default — in effect when no content interpretation OBU is
    // present — is "ci_scan_type_idc = 0 (unspecified)", which satisfies no
    // row. Warning severity, never an error.
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

    // Negative twin: an in-scope content interpretation established a non-zero
    // ci_scan_type_idc, so the coded video sequence flushes without a warning.
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
    // A CLK ends the coded video sequence (AV2 § 7.3.6), retiring the global
    // bucket's scan-type observations: the unestablished-CI warning fires at
    // the restart, not only at the end of the stream, and exactly once (the
    // retired scope leaves nothing for the end-of-stream flush).
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
    // § 6.14 allows different embedded layers to carry different
    // ci_scan_type_idc ("No such constraint exists for content
    // interpretation OBUs in different embedded layers" beyond timing), so a
    // stream with only conforming CI OBUs can establish a matching value at
    // mlayer 0 and a mismatching one at mlayer 1: the later CI must still be
    // paired with the stored observation.
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
    // The global scan-type bucket pairs with every extended layer's CI
    // records, and § 6.14 leaves cross-extended-layer CI content
    // unconstrained (timing aside): a matching CI on xlayer 0 must not stop
    // the later mismatching CI on xlayer 1 from being paired.
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
    // mps_pic_struct_type 7 (Table 6.18: "ci_scan_type_idc shall be equal to
    // 1 and equal_picture_interval shall be equal to 1"): the first CI
    // (matching scan type, no timing) must not stop the later mlayer-1 CI —
    // whose timing_info() signals equal_picture_interval 0 — from being
    // paired with the stored observation.
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
    // A same-key CI repeat with different information is itself
    // non-conforming (§ 6.14, content-interpretation/repeated-ci-not-identical)
    // AND its changed ci_scan_type_idc violates the stored observation's
    // Table 6.18 restriction — distinct rules from distinct spec sections,
    // so both are reported.
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

// --- § 7.3.8.11 CI-parameter epoch at random access points (CLK / OLK) ---

/// An `OBU_OPEN_LOOP_KEY` for xlayer 0 with an empty payload (the raw
/// OBU-header event is all the § 7.3.8.11 epoch tracking consumes).
pub(in crate::validator::tests) fn open_loop_key_obu() -> Vec<u8> {
    annex_b_obu(0x14, &[])
}

#[test]
fn scan_type_pre_olk_ci_not_paired_with_post_olk_metadata() {
    // § 7.3.8.11: the content interpretation parameters re-initialize to
    // defaults (ci_scan_type_idc = 0, unspecified) "at each temporal unit
    // containing an OBU in the extended layer with obu_type equal to
    // OBU_CLOSED_LOOP_KEY or OBU_OPEN_LOOP_KEY". After the OLK with no
    // re-sent CI, the parameters the metadata's pictures see are the
    // defaults — never an error (the unestablished case is warning-only) —
    // so pairing the pre-OLK ci_scan_type_idc 2 against mps_pic_struct_type
    // 0 would be a false positive.
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
    // A CI OBU present in the random access point's own temporal unit
    // re-establishes the parameters (§ 7.3.8.11 step 2), so the Table 6.18
    // pairing fires for post-OLK metadata; the identical re-send is also not
    // a § 6.14 repeated-CI violation.
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
    // The complementary direction: a pre-OLK picture's parameters belong to
    // the previous § 7.3.8.11 epoch, so a CI in the OLK's temporal unit must
    // not be paired with the earlier observation — in either
    // same-temporal-unit order (a CI before the OLK defers the pairing,
    // which the OLK then drops; a CI after the OLK is epoch-skipped).
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
    // The § 6.16.10 Table 6.18 analogue of
    // `metadata_timecode_n_frames_rap_resent_identical_ci_repairs`: the
    // `decisive_content_unchanged` CI dedup guard must be temporal-unit-identity
    // aware, not content-equality-only. A pre-RAP CI at TU0 establishes
    // ci_scan_type_idc 2 (Table 6.18 SingleField). A later RAP temporal unit
    // holds, in decoding order, scan-type metadata mps_pic_struct_type 0 (Frame
    // group, requires ci_scan_type_idc 1) that violates the established 2, then
    // the SAME CI re-sent with ci_scan_type_idc IDENTICAL to the pre-RAP copy,
    // then the CLK (a § 7.3.8.11 random access point). The eager Table 6.18
    // pairing of the metadata against the stale pre-RAP CI is deferred. When the
    // identical CI is re-sent the pre-RAP record is still present (the CLK has
    // not yet pruned it), so a content-equality-only dedup guard would skip the
    // recheck; the CLK then drops the deferred pre-RAP pairing
    // (`drop_pending_for_rules`) — and with the recheck skipped, nothing re-pairs
    // the observation against the post-epoch CI, so the violation vanishes. The
    // re-sent CI is the § 7.3.8.11 authority for this RAP temporal unit's
    // pictures and MUST re-pair the metadata regardless of the idc matching the
    // pre-epoch copy's: the temporal-unit-identity guard re-pairs and the
    // diagnostic fires.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: the pre-RAP CI establishes ci_scan_type_idc 2.
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    // TU1: scan-type metadata (Frame group, requires idc 1) violating the
    // established idc 2 -> the SAME CI re-sent identical (still before the RAP, so
    // the pre-RAP record is the dedup baseline) -> CLK (§ 7.3.8.11 RAP, drops the
    // deferred pre-RAP pairing).
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
    // The § 6.16.10 Table 6.18 analogue of
    // `metadata_timecode_n_frames_rap_resent_identical_ci_before_timecode_reports_once`
    // (the eager-emission-aware RAP re-pair must not duplicate an eagerly-emitted
    // pairing). A pre-RAP CI at TU0 establishes ci_scan_type_idc 2 (Table 6.18
    // SingleField). The RAP temporal unit holds, in decoding order, the SAME CI
    // re-sent identical FIRST, then scan-type metadata mps_pic_struct_type 0 (Frame
    // group, requires ci_scan_type_idc 1) that violates the established 2, then the
    // CLK. Because the re-sent CI is recorded BEFORE the scan-type metadata, the
    // eager metadata-time Table 6.18 check pairs against that same-temporal-unit CI
    // and emits the diagnostic right away (defer_or_emit emits eagerly within one
    // temporal unit). The CLK's repair hook re-pairs the suppressed re-send — but
    // this same-RAP-TU observation was already paired-and-emitted, so it must be
    // skipped: exactly ONE diagnostic. Pre-fix the repair re-paired every post-epoch
    // observation, emitting the mismatch TWICE. (Contrast
    // `scan_type_rap_resent_identical_ci_repairs`, where the metadata precedes the
    // re-sent CI: there the eager pairing DEFERS against the stale pre-RAP CI and is
    // dropped at the RAP, so the repair is the sole source of the one diagnostic and
    // must still fire.)
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: the pre-RAP CI establishes ci_scan_type_idc 2.
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    // TU1: the SAME CI re-sent identical FIRST -> scan-type metadata (Frame group,
    // requires idc 1) violating the established idc 2 -> CLK (§ 7.3.8.11 RAP).
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
    // The OLK analogue of `scan_type_rap_resent_identical_ci_repairs`. An OLK is also
    // a § 7.3.8.11 random access point whose observe_ci_rap advances the epoch and
    // drops the deferred Table 6.18 pairing, so the epoch-aware dedup guard must
    // re-pair against the CI re-sent in the OLK's temporal unit exactly as it does at
    // a CLK. A pre-RAP CI at TU0 establishes ci_scan_type_idc 2 (Table 6.18
    // SingleField). The OLK temporal unit holds, in decoding order, scan-type metadata
    // mps_pic_struct_type 0 (Frame group, requires ci_scan_type_idc 1) violating the
    // established 2, then the SAME CI re-sent with ci_scan_type_idc IDENTICAL to the
    // pre-RAP copy, then the OLK (§ 7.3.8.11 RAP). The eager Table 6.18 pairing
    // against the stale pre-RAP CI is deferred; the identical re-send is skipped by
    // the content-equality dedup guard (pre-RAP record still present); the OLK drops
    // the deferred pre-RAP pairing. With repair wired at the OLK the re-sent CI
    // re-pairs the metadata against the post-epoch authority and the diagnostic fires.
    // (Pre-fix the OLK branch did not call repair_post_rap_ci_pairings, so the
    // violation vanished.)
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    // TU0: the pre-RAP CI establishes ci_scan_type_idc 2.
    data.extend(content_interpretation_scan_obu(2, None));
    data.extend(temporal_delimiter_obu()); // -> TU1, the RAP temporal unit
    // TU1: scan-type metadata (Frame group, requires idc 1) violating the
    // established idc 2 -> the SAME CI re-sent identical (still before the RAP, so
    // the pre-RAP record is the dedup baseline) -> OLK (§ 7.3.8.11 RAP, drops the
    // deferred pre-RAP pairing).
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
    // § 6.14 / § 7.3.8.10 scope the repeated-CI identity rule to the coded
    // video sequence ("all instances of a content interpretation OBU in an
    // embedded layer within a coded video sequence shall contain the same
    // information"), and an OLK does not start one during sequential
    // decoding (§ 7.4.4): the differing repeat across the OLK is still
    // flagged.
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
    // § 6.4.12 binds the timing values "within a coded video sequence ...
    // across all embedded layers"; the OLK is not a CVS boundary during
    // sequential decoding (§ 7.4.4), so the cross-embedded-layer mismatch
    // across it is still flagged.
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

// --- metadata temporal-unit ordering (AV2 § 6.16.3 / § 7.3.7) ---

#[test]
fn metadata_prefix_global_after_coded_layer_is_flagged() {
    // Global prefix metadata (metadata_is_suffix == 0) after a coded extended layer
    // unit is a § 7.3.7 prefix-after-coded-layer violation.
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[])); // coded layer
    // first byte 0x08 = is_suffix 0, cancel 1.
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
    // Global suffix metadata (metadata_is_suffix == 1) after a coded layer is NOT a
    // global prefix, so it must not be flagged.
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[])); // coded layer
    // first byte 0x88 = is_suffix 1, cancel 1.
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
    // Non-global metadata participates in the coded extended layer ascending order:
    // after coded layers at xlayer 0 then 1, a metadata OBU at xlayer 0 is out of
    // order.
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
    data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
    // A cancelled short metadata OBU at xlayer 0 (active sequence 0 is present).
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
