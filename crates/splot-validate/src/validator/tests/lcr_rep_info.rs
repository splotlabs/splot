// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn lcr_rep_info_width_mismatch_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15, // width 16
        height_minus_1: 7, // height 8
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/rep-info-mismatch"),
        "report was: {report}"
    );
    assert!(
        report.errors().any(
            |d| d.rule_id == "lcr/rep-info-mismatch" && d.message.contains("lcr_max_pic_width")
        ),
        "the message must name the width field; report was: {report}"
    );
}

#[test]
fn lcr_rep_info_height_bit_depth_chroma_mismatches_are_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(
        0,
        5,
        16,
        999,          // wrong height
        Some((1, 2)), // wrong bit depth + chroma
        None,
    ));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15, // width 16 (agrees)
        height_minus_1: 7, // height 8
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "lcr/rep-info-mismatch"
                && d.message.contains("lcr_max_pic_height")),
        "height mismatch must be named; report was: {report}"
    );
    assert!(
        report.errors().any(
            |d| d.rule_id == "lcr/rep-info-mismatch" && d.message.contains("lcr_bit_depth_idc")
        ),
        "bit-depth mismatch must be named; report was: {report}"
    );
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "lcr/rep-info-mismatch"
                && d.message.contains("lcr_chroma_format_idc")),
        "chroma mismatch must be named; report was: {report}"
    );
}

#[test]
fn lcr_rep_info_cropping_present_flag_mismatch_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(
        0,
        5,
        16,
        8,
        None,
        Some((0, 0, 0, 0)),
    ));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15,
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None, // present flag 0
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == "lcr/rep-info-mismatch"
            && d.message.contains("lcr_cropping_window_present_flag")),
        "the present-flag disagreement must be named; report was: {report}"
    );
}

#[test]
fn lcr_rep_info_cropping_present_flag_mismatch_also_reports_offsets() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(
        0,
        5,
        16,
        8,
        None,
        Some((1, 0, 0, 0)), // left offset 1, window present
    ));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15,
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None, // present flag 0, offsets inferred 0
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == "lcr/rep-info-mismatch"
            && d.message.contains("lcr_cropping_window_present_flag")),
        "the present-flag disagreement must fire; report was: {report}"
    );
    assert!(
        report.errors().any(|d| d.rule_id == "lcr/rep-info-mismatch"
            && d.message.contains("lcr_cropping_win_left_offset")),
        "the left-offset disagreement must also fire (spec-correct over-reporting); \
         report was: {report}"
    );
}

#[test]
fn lcr_rep_info_cropping_offset_mismatch_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(
        0,
        5,
        16,
        8,
        None,
        Some((1, 2, 9, 4)), // top 9
    ));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15,
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: Some((1, 2, 3, 4)), // top 3
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == "lcr/rep-info-mismatch"
            && d.message.contains("lcr_cropping_win_top_offset")),
        "the offset disagreement must be named; report was: {report}"
    );
}

#[test]
fn lcr_rep_info_full_agreement_passes() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(
        0,
        5,
        16,
        8,
        Some((0, 0)),
        Some((1, 2, 3, 4)),
    ));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15,
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: Some((1, 2, 3, 4)),
    }));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/rep-info-mismatch"),
        "full agreement must be silent; report was: {report}"
    );
}

#[test]
fn lcr_rep_info_absent_format_info_compares_nothing() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(0, 5, 16, 8, None, None));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15,
        height_minus_1: 7,
        chroma_format_idc: 2, // would mismatch if compared
        bit_depth_idc: 1,     // would mismatch if compared
        cropping: None,
    }));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/rep-info-mismatch"),
        "absent format info must compare nothing; report was: {report}"
    );
}

#[test]
fn lcr_rep_info_absent_rep_info_compares_nothing() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu(0, 0, 5, None)); // no rep info
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15,
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/rep-info-mismatch"),
        "absent rep info must compare nothing; report was: {report}"
    );
}

#[test]
fn lcr_rep_info_global_record_is_checked() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu_with_rep_info(5, 0, 1920, 8, None, None));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15,
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(
            |d| d.rule_id == "lcr/rep-info-mismatch" && d.message.contains("lcr_max_pic_width")
        ),
        "the global rep info must be checked; report was: {report}"
    );
}

#[test]
fn lcr_rep_info_not_duplicated_across_reactivation() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15, // width 16 != lcr 1920
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "lcr/rep-info-mismatch"),
        1,
        "report was: {report}"
    );
}

#[test]
fn lcr_rep_info_diagnostic_points_at_lcr_obu() {
    let td = temporal_delimiter_obu();
    let lcr = local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None);
    let seq_start = (td.len() + lcr.len()) as u64;
    let mut data = td;
    data.extend(lcr);
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15, // width 16 != lcr 1920
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    let offsets: Vec<_> = report
        .errors()
        .filter(|d| d.rule_id == "lcr/rep-info-mismatch")
        .map(|d| d.byte_offset)
        .collect();
    assert!(
        matches!(offsets.as_slice(), [Some(offset)] if offset.get() < seq_start),
        "the diagnostic must point at the LCR OBU (before byte {seq_start}); report: {report}"
    );
}

#[test]
fn lcr_rep_info_unconfirmed_activation_is_silent_then_fires_on_frame() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 0, // no LCR association -> not violating
        width_minus_1: 15,
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 1,
        seq_lcr_id: 5,
        width_minus_1: 15, // width 16 != lcr 1920
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    let staged = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&staged, "lcr/rep-info-mismatch"),
        "an unconfirmed activation must be silent; report was: {staged}"
    );
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1));
    let confirmed = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&confirmed, "lcr/rep-info-mismatch"),
        "the frame-confirmed activation must fire; report was: {confirmed}"
    );
}

#[test]
fn lcr_rep_info_redefinition_rechecks_affected_layer() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(0, 5, 16, 8, None, None)); // agrees
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15,
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None)); // redefinition: width 1920
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15,
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "lcr/rep-info-mismatch"),
        1,
        "the redefinition must re-check exactly once; report was: {report}"
    );
}

#[test]
fn lcr_rep_info_redefinition_of_only_dims_rechecks_all_layers_using_the_id() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(0, 5, 16, 8, None, None)); // xlayer 0 LCR, width 16
    data.extend(seq_header_rep_obu_for_xlayer(
        0,
        SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15, // width 16 (agrees with xlayer 0 LCR)
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        },
    ));
    data.extend(local_lcr_obu_with_rep_info(1, 5, 16, 8, None, None)); // xlayer 1 LCR, width 16
    data.extend(seq_header_rep_obu_for_xlayer(
        1,
        SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 15, // width 16 (agrees with xlayer 1 LCR)
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        },
    ));
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 ref seq 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 0)); // CLK xlayer 1 ref seq 0
    data.extend(temporal_delimiter_obu());
    data.extend(seq_header_rep_obu_for_xlayer(
        0,
        SeqRep {
            seq_header_id: 0,
            seq_lcr_id: 5,
            width_minus_1: 7, // width 8 != LCR width 16
            height_minus_1: 7,
            chroma_format_idc: 0,
            bit_depth_idc: 0,
            cropping: None,
        },
    ));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // re-activate seq 0 (xlayer 0)
    let report = Validator::new(false).validate_bytes(&data);
    let xlayer_1_mismatch = report.errors().any(|d| {
        d.rule_id == "lcr/rep-info-mismatch"
            && d.spec_section.as_deref() == Some("6.8.8")
            && d.message.contains("extended layer 1")
    });
    assert!(
        xlayer_1_mismatch,
        "a redefinition changing only max_frame_width must re-run the § 6.8.8 \
         agreement check for every extended layer the id is active for, including the \
         non-activating xlayer 1; report was: {report}"
    );
}

#[test]
fn lcr_rep_info_suppressed_under_external_hls_provided() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15, // width 16 != lcr_max_pic_width 1920
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    assert!(
        has_error(
            &Validator::new(false).validate_bytes(&data),
            "lcr/rep-info-mismatch"
        ),
        "the in-band rep-info mismatch must fire under Disabled"
    );
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "lcr/rep-info-mismatch"),
        "an empty Provided set must suppress the association-dependent rep-info check; \
         report was: {report}"
    );
}

#[test]
fn lcr_rep_info_suppressed_under_ops_only_external_hls_provided() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_rep_info(0, 5, 1920, 8, None, None));
    data.extend(seq_header_rep_payload(SeqRep {
        seq_header_id: 0,
        seq_lcr_id: 5,
        width_minus_1: 15, // width 16 != lcr_max_pic_width 1920
        height_minus_1: 7,
        chroma_format_idc: 0,
        bit_depth_idc: 0,
        cropping: None,
    }));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(0, 3),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "lcr/rep-info-mismatch"),
        "an OPS-only Provided set must suppress the association-dependent rep-info \
         check; report was: {report}"
    );
}

#[test]
fn ops_not_checked_against_ambiguous_fallback_header() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1))); // id 0
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_mlayer_dep_cleared(1),
    ));
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1)); // loads id 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "ops/mlayer-dependency-missing"),
        "the OPS pairs with the frame-confirmed header, not the fallback; report was: {report}"
    );
}

#[test]
fn ops_checked_when_frame_confirms_the_fallback_header() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1))); // id 0
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_mlayer_dep_cleared(1),
    ));
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // loads id 0
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "ops/mlayer-dependency-missing"),
        1,
        "frame confirmation of the violating header must fire once; report was: {report}"
    );
}

/// A sequence header (xlayer 0) with `max_mlayer_id == 1` whose default § 5.4.1
/// dependency fill leaves `MLayerDependencyMap[1][0] == 1` (lower-triangular), so a
/// SWITCH / RAS frame at obu_mlayer_id 1 depends on embedded layer 0.
pub(in crate::validator::tests) fn td_and_seq_header_mlayer_dependent() -> Vec<u8> {
    td_and_seq_header(0, 0, 1)
}

#[test]
fn switch_frame_depending_on_another_embedded_layer_is_flagged() {
    let mut data = td_and_seq_header_mlayer_dependent();
    data.extend(frame_obu_direct_seq_ref_layer(10, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/switch-or-ras-mlayer-dependency-not-self-contained"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "report was: {report}"
    );
}

#[test]
fn ras_frame_depending_on_another_embedded_layer_is_flagged() {
    let mut data = td_and_seq_header_mlayer_dependent();
    data.extend(frame_obu_direct_seq_ref_layer(21, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/switch-or-ras-mlayer-dependency-not-self-contained"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "report was: {report}"
    );
}

#[test]
fn self_contained_ras_frame_is_not_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_mlayer_dep_cleared(0),
    ));
    data.extend(frame_obu_direct_seq_ref_layer(21, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id
                == "frame-header/switch-or-ras-mlayer-dependency-not-self-contained"),
        "a self-contained map must not flag the RAS frame; report was: {report}"
    );
}

#[test]
fn switch_frame_at_base_embedded_layer_is_not_flagged() {
    let mut data = td_and_seq_header_mlayer_dependent();
    data.extend(frame_obu_direct_seq_ref_layer(10, 0, 0, 0, 0)); // SWITCH, mlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id
                == "frame-header/switch-or-ras-mlayer-dependency-not-self-contained"),
        "report was: {report}"
    );
}

/// A sequence header (xlayer 0) with `max_mlayer_id == 1` and
/// `seq_max_mlayer_cnt_minus_1 == 0` (`SeqMaxMlayerCnt == 1`): only one distinct
/// embedded layer is allowed in the coded video sequence, even though embedded layer
/// 1 is otherwise within `max_mlayer_id`.
pub(in crate::validator::tests) fn seq_header_payload_seqmaxcnt_one() -> Vec<u8> {
    seq_header_payload_seqmaxcnt_one_id(0)
}

/// As [`seq_header_payload_seqmaxcnt_one`] but with an explicit `seq_header_id`, so a
/// fixture can place two distinct SeqMaxMlayerCnt-1 headers (e.g. an outgoing header
/// and a different one a CLK re-references) in the same stream.
pub(in crate::validator::tests) fn seq_header_payload_seqmaxcnt_one_id(
    seq_header_id: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(seq_header_id); // seq_header_id
    bits.f(0, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    bits.f(0, 5); // seq_level_idx
    bits.uvlc(0); // chroma_format_idc
    bits.uvlc(0); // bit_depth_idc
    bits.f(0, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(0, 2); // max_tlayer_id
    bits.f(1, 3); // max_mlayer_id = 1
    bits.f(0, 1); // seq_max_mlayer_cnt_minus_1 = 0 -> SeqMaxMlayerCnt = 1
    bits.bit(1); // monotonic_output_order_flag
    bits.f(3, 4); // frame_width_bits_minus_1
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(15, 4); // max_frame_width_minus_1
    bits.f(7, 4); // max_frame_height_minus_1
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    bits.bit(0); // mlayer_dependency_present_flag (max_mlayer_id > 0)
    append_non_single_child_configs(&mut bits);
    bits.into_bytes()
}

/// A sequence-header payload with `max_mlayer_id == 2` and `SeqMaxMlayerCnt == 2`
/// (`seq_max_mlayer_cnt_minus_1 == 1`): the coded video sequence may use embedded
/// layers up to 2 but at most two *distinct* `obu_mlayer_id` values (AV2 § 6.4.1).
pub(in crate::validator::tests) fn seq_header_payload_seqmaxcnt_two() -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id
    bits.f(0, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    bits.f(0, 5); // seq_level_idx
    bits.uvlc(0); // chroma_format_idc
    bits.uvlc(0); // bit_depth_idc
    bits.f(0, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(0, 2); // max_tlayer_id
    bits.f(2, 3); // max_mlayer_id = 2
    bits.f(1, ceil_log2_u32(3)); // seq_max_mlayer_cnt_minus_1 = 1 -> SeqMaxMlayerCnt = 2
    bits.bit(1); // monotonic_output_order_flag
    bits.f(3, 4); // frame_width_bits_minus_1
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(15, 4); // max_frame_width_minus_1
    bits.f(7, 4); // max_frame_height_minus_1
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    bits.bit(0); // mlayer_dependency_present_flag (max_mlayer_id > 0)
    append_non_single_child_configs(&mut bits);
    bits.into_bytes()
}
