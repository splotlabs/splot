// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn active_sequence_header_allows_following_obu_within_layer_limits() {
    let mut data = stream_with_sequence_header(1, 1);
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 0), &[]));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("sequence-state/")),
        "report was: {report}"
    );
}

#[test]
fn sequence_header_with_decoder_model_info_tail_can_activate() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_decoder_model_info(),
    ));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 0), &[]));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "bitstream/parse-error"),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("sequence-state/")),
        "report was: {report}"
    );
}

#[test]
fn layer_obu_before_sequence_header_reports_missing_active_sequence() {
    let data = annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
        "report was: {report}"
    );
}

#[test]
fn repeated_sequence_header_does_not_replace_active_limits_without_reference() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 0), &[]));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("sequence-state/")),
        "report was: {report}"
    );
}

#[test]
fn local_prefix_hls_before_sequence_header_does_not_require_active_sequence() {
    for obu_type in [16, 17, 18] {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(obu_type, 0, 0, 0),
            &[],
        ));
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
            "obu_type={obu_type}, report was: {report}"
        );
    }
}

#[test]
fn non_activating_sequence_header_does_not_suppress_missing_active_sequence_error() {
    let mut data = annex_b_obu(0x05, &sequence_header_payload(1, 0));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
        "report was: {report}"
    );
}

#[test]
fn temporal_unit_accepts_ascending_coded_xlayers() {
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 0), &[]));
    data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 1), &[]));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id.starts_with("obu-order/")),
        "report was: {report}"
    );
    assert!(report.is_conformant(), "report was: {report}");
}

#[test]
fn global_hls_in_prefix_phase_is_accepted() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(16, 0, 0, 31),
        &[],
    ));
    data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id.starts_with("obu-order/")),
        "report was: {report}"
    );
}

#[test]
fn temporal_unit_missing_delimiter_is_reported() {
    let data = annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-order/temporal-unit-missing-delimiter"),
        "report was: {report}"
    );
}

#[test]
fn global_hls_after_coded_layer_is_reported() {
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(16, 0, 0, 31),
        &[],
    ));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-order/global-hls-after-coded-layer"),
        "report was: {report}"
    );
}

#[test]
fn coded_xlayers_must_ascend_within_temporal_unit() {
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 1), &[]));
    data.extend(sequence_header_obu_for_xlayer(0, 1, 1));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-order/xlayer-order-not-ascending"),
        "report was: {report}"
    );
}

#[test]
fn non_global_padding_outside_coded_layer_is_reported() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(&layer_obu_header(25, 0, 0, 0), &[]));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-order/padding-non-global-outside-coded-layer"),
        "report was: {report}"
    );
}

#[test]
fn active_sequence_header_bounds_temporal_layer_id() {
    let mut data = stream_with_sequence_header(1, 1);
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 2, 0, 0), &[]));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
        "report was: {report}"
    );
}

#[test]
fn active_sequence_header_bounds_embedded_layer_id() {
    let mut data = stream_with_sequence_header(1, 1);
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 2, 0), &[]));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-state/mlayer-exceeds-max"),
        "report was: {report}"
    );
}

#[test]
fn redefinition_tightening_before_reconfirming_frame_is_not_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 3, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame-confirm id 0 (max 3)
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 0))); // redefine: max 1
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 2, 0, 0), &[])); // tlayer 2, pre-CLK
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK re-confirms id 0

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
        "obu_tlayer_id 2 between a tightened redefinition (max 1) and its re-confirming CLK \
         frame conforms to the in-force activated max 3 (§ 6.2.2 NOTE); report was: {report}"
    );
}

#[test]
fn redefinition_before_reconfirming_frame_still_flags_old_limit_violation() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 2, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame-confirm id 0 (max 2)
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 0))); // redefine: max 1
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 3, 0, 0), &[])); // tlayer 3 > prior 2
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK re-confirms id 0

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/tlayer-exceeds-max"
                && d.spec_section.as_deref() == Some("6.2.2")
        }),
        "obu_tlayer_id 3 exceeds the prior activated max 2 and must still fire; report was: \
         {report}"
    );
}

#[test]
fn tightened_limit_applies_after_reconfirming_frame() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 3, 0)));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame-confirm id 0 (max 3)
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 0))); // redefine: max 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK re-confirms id 0 (max 1)
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 2, 0, 0), &[])); // tlayer 2 > new 1

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/tlayer-exceeds-max"
                && d.spec_section.as_deref() == Some("6.2.2")
        }),
        "after the CLK re-confirms the tightened header (max 1), obu_tlayer_id 2 must fire; \
         report was: {report}"
    );
}

#[test]
fn redefinition_window_padding_carrier_full_stream_is_conformant() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 3, 0))); // L_old, max_tlayer 3
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK starts CVS_k, activates L_old (max 3)
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 0))); // L_new redef, max_tlayer 1
    data.extend(annex_b_obu_with_header(&layer_obu_header(25, 2, 0, 0), &[])); // OBU_PADDING X, tlayer 2
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK starts CVS_(k+1), re-activates L_new

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.is_conformant(),
        "the pre-reactivation padding (obu_tlayer_id 2) is bounded by the prior activated \
         max_tlayer_id 3 (§ 6.2.2 NOTE), so the whole stream must be conformant; report was: \
         {report}"
    );
}

#[test]
fn stateful_diagnostics_from_prefix_survive_a_later_parse_error() {
    let mut data = stream_with_sequence_header(0, 0);
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 0, 0), &[]));
    data.extend_from_slice(&[0x05, 0x08]);

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
        "report was: {report}"
    );
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "bitstream/parse-error"),
        "report was: {report}"
    );
}
