// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn ops_deferred_check_fires_on_frame_activation_change() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_mlayer_dep_cleared(0),
    ));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 1)));
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "ops/mlayer-dependency-missing"),
        1,
        "the frame-driven re-activation must evaluate stored OPS maps; report was: {report}"
    );
}

#[test]
fn ops_disagreement_reemitted_after_sequence_header_redefinition() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(local_ops_mlayer_obu(0, 0, 0b10, &[0b1]));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 1)));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "ops/mlayer-dependency-missing"),
        2,
        "a same-id redefinition must re-fire the agreement checks; report was: {report}"
    );
}

#[test]
fn lcr_local_tlayer_dependency_missing_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b1, &[0b10]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/tlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn lcr_global_mlayer_dependency_missing_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(global_lcr_obu_with_embedded(5, 3, 0b10, &[0b1]));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(1, 0, 0, 3),
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 3, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/mlayer-dependency-missing"),
        "report was: {report}"
    );
}

#[test]
fn lcr_local_record_takes_precedence_over_global() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b11, &[0b11, 0b11]));
    data.extend(global_lcr_obu_with_embedded(5, 0, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "the local LCR resolves first (§ 6.4.1); report was: {report}"
    );
}

#[test]
fn lcr_unresolved_nonzero_seq_lcr_id_is_not_dependency_checked() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "hls/unavailable-layer-configuration-record"),
        "report was: {report}"
    );
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "an unresolved seq_lcr_id must not be dependency-checked; report was: {report}"
    );
}

#[test]
fn lcr_after_sequence_header_is_not_paired() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "a later LCR must not pair with an earlier activation; report was: {report}"
    );
}

#[test]
fn lcr_redefined_without_embedded_info_is_not_checked() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(local_lcr_obu(0, 0, 5, None)); // redefinition, no embedded info
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "stale maps from a superseded definition must not be checked; report was: {report}"
    );
}

#[test]
fn lcr_dependency_check_suppressed_under_external_hls_provided() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let baseline = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&baseline, "lcr/mlayer-dependency-missing"),
        "the in-band violation must fire under Disabled; report was: {baseline}"
    );
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "any Provided external HLS must suppress the association-dependent LCR \
         dependency check (an unmodeled external local LCR could shadow the in-band \
         association); report was: {report}"
    );
}

#[test]
fn lcr_dependency_uses_strict_frame_confirmation() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "a sole staged header (no frame) must not fire the LCR dependency check via the \
         sole-header fallback; report was: {report}"
    );
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "lcr/mlayer-dependency-missing"),
        "the frame-confirmed activation must fire the LCR dependency check; report was: \
         {report}"
    );
}

#[test]
fn lcr_ptl_uses_strict_frame_confirmation() {
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_ptl(0, 5, 0, 4, 0, 1));
    data.extend(seq_header_ptl_payload(SeqPtl {
        seq_header_id: 0,
        seq_lcr_id: 5,
        profile: 0,
        level: 8, // > lcr_max_level_idx 4
        tier: 0,
        max_mlayer_id: 0,
    }));
    assert!(
        !has_error(
            &Validator::new(false).validate_bytes(&data),
            "lcr/ptl-level-exceeds-max"
        ),
        "a sole staged header (no frame) must not fire the § 6.8.5 ceiling via the \
         sole-header fallback"
    );
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    assert!(
        has_error(
            &Validator::new(false).validate_bytes(&data),
            "lcr/ptl-level-exceeds-max"
        ),
        "the frame-confirmed activation must fire the § 6.8.5 ceiling"
    );
}

#[test]
fn lcr_agreement_silent_when_external_header_could_be_the_activator() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(9)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "with an external header declared and no frame, the LCR check must not fire \
         against a guessed in-band activation; report was: {report}"
    );
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "under any Provided mode the association-dependent LCR check stays suppressed; \
         report was: {report}"
    );
}

#[test]
fn lcr_repeated_sequence_header_pairs_with_now_present_lcr() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(0, 5, 1, 1),
    ));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        ops_error_count(&report, "lcr/mlayer-dependency-missing"),
        1,
        "the repeated header must pair with the now-present LCR; report was: {report}"
    );
}

#[test]
fn lcr_association_snapshotted_at_header_observation() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload(1, 1))); // id 0
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b11, &[0b11, 0b11]));
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_lcr(1, 5, 1, 1), // id 1, seq_lcr_id 5
    ));
    data.extend(local_lcr_obu_with_embedded(0, 5, 0b10, &[0b1])); // redefinition
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1)); // activates id 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "lcr/mlayer-dependency-missing")
            && !has_error(&report, "lcr/tlayer-dependency-missing"),
        "the post-header redefinition must not be paired with header 1; report was: {report}"
    );
}
