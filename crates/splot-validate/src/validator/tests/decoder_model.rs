// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn decoder_model_intra_cvs_ops_sum_change_is_error() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "an intra-CVS OPS buffer-delay sum change must be a single error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_sum_change_before_first_clk_is_not_error() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30, no CVS yet
    data.extend(local_ops_obu_with_delays(2, false, 0, 25, 15)); // sum 40, still no CVS
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a pre-first-CLK OPS sum change is in no coded video sequence and must not \
         be an error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "a pre-first-CLK OPS sum change spans no boundary and must not warn: {report}"
    );
}

#[test]
fn decoder_model_ops_sum_change_with_late_clk_in_same_tu_is_not_error() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // TU1: starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, CVS 1
    data.extend(temporal_delimiter_obu()); // TU2 begins
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, epoch still 1
    data.extend(clk_frame_for_xlayer(0, 0)); // late CLK -> TU2 is CVS 2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a late same-TU CLK makes the change cross-CVS; the deferred error must be \
         dropped: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "the dropped deferred error must be replaced by the cross-CVS advisory, not \
         silently lost: {report}"
    );
}

#[test]
fn decoder_model_intra_cvs_ops_same_sum_is_not_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu_with_delays(2, false, 0, 20, 10)); // sum 30
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "an unchanged sum must not be flagged: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "an unchanged sum must not raise the advisory either: {report}"
    );
}

#[test]
fn decoder_model_ops_sum_change_across_cvs_is_not_error_but_warns() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // TU1: starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, CVS 1
    data.extend(temporal_delimiter_obu()); // TU2 begins
    data.extend(clk_frame_for_xlayer(0, 0)); // TU2: starts CVS 2 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, CVS 2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a cross-CVS OPS sum change must not be an error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "a cross-CVS OPS sum change must raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_sum_change_across_reset_is_not_error_but_warns() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu_with_delays(2, true, 0, 25, 15)); // reset, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a reset-spanning OPS sum change must not be an error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "a reset-spanning OPS sum change must raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_redefinition_without_explicit_info_clears_baseline() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu_with_delays(2, false, 0, 10, 20)); // sum 30, explicit
    data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // no decoder-model info
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "an absent-info redefinition clears the baseline and must not be compared: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "an absent-info redefinition must not raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_annex_e_defaults_are_never_compared() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // no decoder-model info
    data.extend(local_ops_obu_with_delays(2, false, 0, 70_000, 20_000)); // explicit 90000
    data.extend(local_ops_obu(2, false, 0, 1, 0, false, 0)); // no decoder-model info
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "absent-info OPS using the Annex E defaults must not be compared: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "absent-info OPS must not raise the advisory against an explicit value: {report}"
    );
}

#[test]
fn decoder_model_seq_header_sum_change_across_cvs_warns() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    )); // sum 0
    data.extend(clk_frame_for_xlayer(0, 0)); // confirm + start CVS 1
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
    )); // sum 12
    data.extend(clk_frame_for_xlayer(0, 1)); // confirm + start CVS 2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "an activated seq-header sum change across a CLK must raise the advisory: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "the seq-header tier is advisory only, never an error: {report}"
    );
}

#[test]
fn decoder_model_seq_header_same_id_reconfiguration_across_cvs_warns() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    )); // id 0, sum 0
    data.extend(clk_frame_for_xlayer(0, 0)); // confirm id 0 + start CVS 1
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 5, 7),
    )); // id 0 again, sum 12
    data.extend(clk_frame_for_xlayer(0, 0)); // re-confirm id 0 + start CVS 2
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "a same-id reconfiguration changing the sum across a CVS boundary must raise \
         the advisory: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "the seq-header tier is advisory only, never an error: {report}"
    );
}

#[test]
fn decoder_model_seq_header_without_info_never_warns() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "headers without decoder-model info must not raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_seq_header_fallback_guess_activation_never_warns() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    ));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "an unconfirmed fallback-guess activation must not participate: {report}"
    );
}

#[test]
fn decoder_model_external_hls_suppresses_both_ids() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    ));
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40 (intra-CVS)
    data.extend(clk_frame_for_xlayer(0, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
    ));
    data.extend(clk_frame_for_xlayer(0, 1));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new()
                .with_sequence_header_id(0)
                .with_sequence_header_id(1),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "external HLS must suppress the OPS error tier: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "external HLS must suppress both decoder-model advisories: {report}"
    );
}

#[test]
fn decoder_model_external_hls_without_seq_headers_still_suppresses_seq_advisory() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    ));
    data.extend(clk_frame_for_xlayer(0, 0)); // confirm + start CVS 1
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(1, 5, 7),
    )); // differing sum
    data.extend(clk_frame_for_xlayer(0, 1)); // confirm + start CVS 2
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new().with_operating_point_set(31, 0),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "Provided external HLS without declared sequence headers must still suppress \
         the seq-header advisory: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "the seq-header tier never emits an error: {report}"
    );
}

#[test]
fn decoder_model_ops_sum_change_across_targeted_reset_is_not_error_but_warns() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu(0, false, 0, 0, 0, false, 0)); // targeted reset of OPS 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // redefine, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a targeted-reset-spanning OPS sum change must not be an error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "a targeted-reset-spanning OPS sum change must raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_targeted_reset_of_other_ops_still_errors() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // OPS 0, sum 30
    data.extend(local_ops_obu(0, false, 1, 0, 0, false, 0)); // targeted reset of OPS 1
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // OPS 0, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "a targeted reset of a different OPS must not excuse OPS 0's intra-CVS sum \
         change: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_dm_less_redefinition_clears_baseline_no_diagnostic() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, explicit
    data.extend(local_ops_obu(0, false, 0, 1, 0, false, 0)); // redefine OPS 0, no dm info
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, explicit
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a dm-less redefinition clears the baseline; explicit-40 must not error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "a cleared baseline must not be compared, so no advisory either: {report}"
    );
}

#[test]
fn decoder_model_ops_unrelated_redefinition_still_errors_within_cvs() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // OPS 0, sum 30
    data.extend(local_ops_obu(0, false, 1, 1, 0, false, 0)); // unrelated OPS 1, no dm info
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // OPS 0, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "a dm-less redefinition of a DIFFERENT OPS must not clear OPS 0's baseline: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_seq_header_dm_less_activation_clears_baseline_no_warning() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(0, 0, 0),
    )); // id 0, sum 0
    data.extend(clk_frame_for_xlayer(0, 0)); // confirm + start CVS 1
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 1))); // id 1, no dm info
    data.extend(clk_frame_for_xlayer(0, 1)); // confirm + start CVS 2 (clears baseline)
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_sum(2, 5, 7),
    )); // id 2, sum 12
    data.extend(clk_frame_for_xlayer(0, 2)); // confirm + start CVS 3
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "a dm-less activation between the two explicit headers clears the baseline; \
         the later explicit sum must not raise the advisory: {report}"
    );
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "the seq-header tier never emits an error: {report}"
    );
}

#[test]
fn decoder_model_ops_cross_layer_local_reset_does_not_excuse_intra_cvs_error() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // xlayer 0, sum 30
    data.extend(local_ops_obu(1, true, 0, 0, 0, false, 0)); // LOCAL reset of xlayer 1
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // xlayer 0, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "a local reset of an unrelated extended layer must not excuse xlayer 0's \
         intra-CVS sum change: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_global_reset_re_baselines_other_layers() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(clk_frame_for_xlayer(0, 0)); // starts CVS 1 for xlayer 0
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // xlayer 0, sum 30
    data.extend(local_ops_obu(31, true, 0, 0, 0, false, 0)); // GLOBAL reset (all layers)
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // xlayer 0, sum 40
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a global reset re-baselines xlayer 0, so the change is not an error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        1,
        "a global-reset-spanning sum change must raise the advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_pre_clk_baseline_in_same_tu_migrates_to_new_cvs_error() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, pre-CLK
    data.extend(clk_frame_for_xlayer(0, 0)); // CLK -> whole TU is the new CVS
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, post-CLK, same TU
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "a pre-CLK baseline in the CLK's own TU migrates to the new CVS; the post-CLK \
         change is intra-CVS and must error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the migrated intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_both_definitions_pre_clk_in_same_tu_is_error() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, pre-CLK
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, pre-CLK
    data.extend(clk_frame_for_xlayer(0, 0)); // CLK later in same TU -> whole TU is the new CVS
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        1,
        "both pre-CLK OPS definitions in the CLK's own temporal unit are intra-CVS; \
         the differing sum must be a single error: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the deferred intra-CVS error must not also raise the cross-CVS advisory: {report}"
    );
}

#[test]
fn decoder_model_ops_both_definitions_pre_clk_no_clk_in_tu_stays_silent() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30, no CVS yet
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40, still no CVS
    data.extend(temporal_delimiter_obu()); // TU closes with no CLK for the layer
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        0,
        "a pre-first-CLK OPS sum change whose temporal unit closes with no CLK is in \
         no coded video sequence and must stay silent: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "the dropped pre-CVS comparison spans no boundary and must not warn: {report}"
    );
}

#[test]
fn decoder_model_ops_multiple_pre_clk_changes_same_tu_error_per_change() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(local_ops_obu_with_delays(0, false, 0, 10, 20)); // sum 30
    data.extend(local_ops_obu_with_delays(0, false, 0, 25, 15)); // sum 40
    data.extend(local_ops_obu_with_delays(0, false, 0, 30, 20)); // sum 50
    data.extend(clk_frame_for_xlayer(0, 0)); // CLK later in same TU
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "decoder-model/buffer-delay-sum-changed"),
        2,
        "two consecutive intra-CVS sum changes must produce two errors, one per \
         comparison: {report}"
    );
    assert_eq!(
        decoder_model_warning_count(&report, "decoder-model/buffer-delay-sum-changed-across-cvs"),
        0,
        "intra-CVS changes must not raise the cross-CVS advisory: {report}"
    );
}
