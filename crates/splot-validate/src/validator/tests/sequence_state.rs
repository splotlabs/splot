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
