// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn distinct_mlayer_count_exceeds_seqmax_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_within_seqmax_is_conforming() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_resets_at_cvs_boundary() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK, mlayer 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "the count must reset at the § 7.3.6 CVS boundary; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_before_first_clk_uses_active_header() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // mlayer 1, no frame ref
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_emits_once_per_cvs() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max")
            .count(),
        1,
        "the § 6.4.1 distinct-mlayer check must emit once per CVS; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_pre_clk_obu_in_boundary_tu_is_not_flagged() {
    let mut data = temporal_delimiter_obu(); // temporal unit 0
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK, mlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK, mlayer 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "a pre-CLK OBU in the CVS-starting temporal unit belongs to the new coded video \
         sequence; the new CVS {{1}} does not exceed and the deferred old-CVS exceedance \
         is dropped; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_pre_clk_header_reattributed_to_new_cvs_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            })
            .count(),
        1,
        "the pre-CLK header is re-attributed to the new CVS; {{0, 1}} = 2 > 1 must fire \
         exactly once; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_reattribution_excludes_pre_boundary_tu_ids() {
    let mut data = temporal_delimiter_obu(); // temporal unit 0
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_two()));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 2, 0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_two())); // resent header, mlayer 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK @ mlayer 1, ref seq 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "the new CVS must re-seed only boundary-temporal-unit ids ({{0, 1}} <= 2); \
         earlier-temporal-unit ids must not count; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_reattribution_reports_once_across_clk_in_boundary_tu() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // header, mlayer 0
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max")
            .count(),
        1,
        "the exceedance visible both pre- and post-CLK in the boundary temporal unit \
         must report exactly once; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_reattribution_compares_against_clk_activated_header() {
    let mut data = temporal_delimiter_obu(); // temporal unit 0
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // header id 0, max 1
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 1)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // re-sent header, mlayer 0
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK OBU, mlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 1)); // CLK mlayer 0, ref seq 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "the re-seeded {{0, 1}} = 2 set must be compared against the CLK-activated header \
         (id 1, SeqMaxMlayerCnt 2), not the outgoing header (id 0, max 1); report was: \
         {report}"
    );
}

#[test]
fn distinct_mlayer_count_reattribution_clk_activated_header_lower_max_is_flagged() {
    let mut data = temporal_delimiter_obu(); // temporal unit 0
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 1)));
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one_id(1))); // header id 1, max 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame @ mlayer 0, ref seq 0
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 1))); // re-sent header id 0, mlayer 0
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK OBU, mlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 1)); // CLK mlayer 0, ref seq 1 (max 1)
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            })
            .count(),
        1,
        "the re-seeded {{0, 1}} = 2 set exceeds the CLK-activated header's max 1 and must \
         fire exactly once; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_reattribution_same_header_exceedance_is_flagged() {
    let mut data = temporal_delimiter_obu(); // temporal unit 0
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // header id 0, max 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame @ mlayer 0, ref seq 0
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one())); // re-sent header, mlayer 0
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[])); // pre-CLK OBU, mlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 0)); // CLK mlayer 1, ref seq 0
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            })
            .count(),
        1,
        "the re-seeded {{0, 1}} = 2 set exceeds the re-referenced header's max 1 and must \
         fire exactly once even though the CLK's own mlayer is already in the set; report \
         was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "external HLS must suppress the § 6.4.1 distinct-mlayer check; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_accumulated_before_header_activation_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            })
            .count(),
        1,
        "a pre-header distinct-mlayer count must fire once on activation; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_accumulated_before_header_activation_within_seqmax_is_conforming() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "a pre-header count within SeqMaxMlayerCnt must not fire; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_before_header_activation_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 0), &[]));
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "external HLS must suppress the retroactive distinct-mlayer check; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_before_frame_header_activation_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 1), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 1), &[]));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| {
                d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"
                    && d.spec_section.as_deref() == Some("6.4.1")
            })
            .count(),
        1,
        "a pre-header count must fire once on frame-header activation; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_before_frame_header_activation_within_seqmax_is_conforming() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(0, 1)));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 1), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 1), &[]));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "a pre-header count within SeqMaxMlayerCnt must not fire; report was: {report}"
    );
}

#[test]
fn distinct_mlayer_count_before_frame_header_activation_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &seq_header_payload_seqmaxcnt_one()));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 0, 1), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(7, 0, 1, 1), &[]));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/distinct-mlayer-count-exceeds-seq-max"),
        "external HLS must suppress the frame-header-path retroactive check; report was: {report}"
    );
}

#[test]
fn second_activation_without_clk_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "hls/multiple-active-sequence-headers"
                && d.spec_section.as_deref() == Some("7.3.6")
        }),
        "report was: {report}"
    );
}

#[test]
fn reactivation_across_clk_is_conforming() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // confirm seq 0
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 1)); // CLK, ref seq 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
        "a CLK re-activation must not fire the § 7.3.6 check; report was: {report}"
    );
}

#[test]
fn fallback_guess_then_frame_reference_is_not_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
        "a fallback guess overridden by the first frame must not fire; report was: {report}"
    );
}

#[test]
fn unreferenced_extra_sequence_header_is_conforming() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
        "an unreferenced extra sequence header must not fire; report was: {report}"
    );
}

#[test]
fn second_activation_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/multiple-active-sequence-headers"),
        "external HLS must suppress the § 7.3.6 check; report was: {report}"
    );
}

/// Builds the otherwise-firing two-activation stream of
/// [`second_activation_without_clk_is_flagged`] (frame-confirm seq 0, then a non-CLK
/// frame activating seq 1 in the same CVS for xlayer 0).
pub(in crate::validator::tests) fn two_activation_stream() -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // confirm seq 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1)); // confirm seq 1
    data
}

#[test]
fn second_activation_under_empty_external_hls_is_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report =
        Validator::new(false).validate_bytes_with_options(&two_activation_stream(), &options);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "hls/multiple-active-sequence-headers"
                && d.spec_section.as_deref() == Some("7.3.6")
        }),
        "an empty external set declares no sequence header and must not suppress; \
         report was: {report}"
    );
}

#[test]
fn second_activation_under_sequence_free_external_hls_is_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(0, 0),
        ),
    };
    let report =
        Validator::new(false).validate_bytes_with_options(&two_activation_stream(), &options);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "hls/multiple-active-sequence-headers"
                && d.spec_section.as_deref() == Some("7.3.6")
        }),
        "a sequence-header-free external set must not suppress; report was: {report}"
    );
}

#[test]
fn second_activation_under_out_of_range_external_hls_id_is_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(16)),
    };
    let report =
        Validator::new(false).validate_bytes_with_options(&two_activation_stream(), &options);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "hls/multiple-active-sequence-headers"
                && d.spec_section.as_deref() == Some("7.3.6")
        }),
        "an out-of-range external id is ignored and must not suppress; report was: {report}"
    );
}

/// A sequence-header payload (xlayer-neutral) with the given `seq_header_id`,
/// `max_mlayer_id == 0`, and an explicit `monotonic_output_order_flag`.
pub(in crate::validator::tests) fn seq_header_payload_monotonic(
    seq_header_id: u32,
    monotonic: bool,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(seq_header_id);
    bits.f(0, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    bits.f(0, 5); // seq_level_idx
    bits.uvlc(0); // chroma_format_idc
    bits.uvlc(0); // bit_depth_idc
    bits.f(0, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(0, 2); // max_tlayer_id
    bits.f(0, 3); // max_mlayer_id = 0 (no seq_max_mlayer_cnt_minus_1 field)
    bits.bit(u8::from(monotonic)); // monotonic_output_order_flag
    bits.f(3, 4); // frame_width_bits_minus_1
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(15, 4); // max_frame_width_minus_1
    bits.f(7, 4); // max_frame_height_minus_1
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    append_non_single_child_configs(&mut bits);
    bits.into_bytes()
}

/// A sequence-header OBU for `xlayer` carrying [`seq_header_payload_monotonic`].
pub(in crate::validator::tests) fn seq_header_obu_monotonic(
    xlayer: u8,
    seq_header_id: u32,
    monotonic: bool,
) -> Vec<u8> {
    let payload = seq_header_payload_monotonic(seq_header_id, monotonic);
    if xlayer == 0 {
        annex_b_obu(0x04, &payload)
    } else {
        annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
    }
}

/// Builds a stream whose temporal unit 1 begins a § 7.3.2 CMVS (begin condition 1:
/// a CLK temporal unit with an MSDO present and no CMVS yet active), with sequence
/// headers for extended layers 0 and 1 whose `monotonic_output_order_flag` values
/// are `monotonic_x0` and `monotonic_x1`. Temporal unit 2 (the CMVS is definitively
/// `Inside` by then) *frame-confirms* both extended layers in turn — first xlayer 0,
/// then xlayer 1 — so each layer is associated with its referenced sequence header
/// per § 5.18.2 rather than the OBU-order fallback guess (§ 7.3.6 forbids treating
/// an unreferenced extra header as activated). The cross-layer agreement check runs
/// at each frame; the disagreement, if any, is emitted "when the second of the two
/// headers is activated" (the xlayer-1 frame).
pub(in crate::validator::tests) fn cmvs_two_layer_stream(
    monotonic_x0: bool,
    monotonic_x1: bool,
) -> Vec<u8> {
    let mut data = temporal_delimiter_obu(); // starts temporal unit 1
    data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO
    data.extend(seq_header_obu_monotonic(0, 0, monotonic_x0)); // xlayer 0 seq 0
    data.extend(seq_header_obu_monotonic(1, 1, monotonic_x1)); // xlayer 1 seq 1
    data.extend(annex_b_obu(0x10, &[])); // CLK xlayer 0 -> begins the CMVS
    data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS now Inside
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1));
    data
}
