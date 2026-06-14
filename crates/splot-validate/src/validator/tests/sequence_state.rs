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
    // 0x05 = OBU_SEQUENCE_HEADER at tlayer=1, so it parses but cannot activate.
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
    // § 6.2.2 NOTE (mirror `06-syntax-structures-semantics.md` lines 197-198): the
    // obu_tlayer_id/obu_mlayer_id <= max constraints apply *after* a sequence header is
    // activated. § 7.3.6 permits a same-seq_header_id redefinition at a coded-video-sequence
    // boundary (a CLK). An OBU sitting between a redefined (tightened) header and the CLK
    // frame that re-activates it is still in the PREVIOUS activation's window: under every
    // decode start its in-force activated max is the prior (looser) one, not the
    // freshly-stored (not-yet-reactivated) tighter one. The limit must be evaluated against
    // the frame-confirmed activated header, not the latest-stored payload.
    //
    // TU0: header id 0 (max_tlayer_id 3) is frame-confirmed by a regular tile group.
    // TU1 (CLK boundary): header id 0 is redefined to max_tlayer_id 1, then a layer OBU at
    // obu_tlayer_id 2 appears BEFORE the CLK frame that re-confirms the redefinition. 2 <= 3
    // (the in-force activated max) so nothing must fire; comparing 2 against the stored-but-
    // not-yet-reactivated max 1 would be a false positive.
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
    // Companion true-positive: the § 6.2.2 NOTE refinement must not over-suppress. An OBU in
    // the same pre-re-confirmation window whose obu_tlayer_id exceeds even the PRIOR activated
    // max is a real violation under the start-from-beginning decode and must still fire,
    // anchored at § 6.2.2. Same shape as above but obu_tlayer_id 3 > prior max 2.
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
    // The refinement compares against the in-force activated header, which the re-confirming
    // CLK frame advances to the redefinition: an OBU in the NEXT temporal unit (after the CLK
    // frame re-activated id 0 with max_tlayer_id 1) at obu_tlayer_id 2 now exceeds the
    // tightened max and must fire. Proves the snapshot tracks re-activation, not a frozen
    // prior limit.
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
