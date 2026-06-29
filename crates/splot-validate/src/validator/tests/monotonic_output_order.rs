// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn monotonic_output_order_disagreement_inside_cmvs_is_flagged() {
    let data = cmvs_two_layer_stream(true, false);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "report was: {report}"
    );
}

/// A single temporal unit that *opens* a § 7.3.2 CMVS (begin condition 1: a CLK
/// temporal unit with an MSDO present and no CMVS yet active) and frame-confirms both
/// extended layers WITHIN that same opening temporal unit. xlayer 0's CLK references
/// seq 0 (`monotonic_x0`); xlayer 1's CLK references seq 1 (`monotonic_x1`). The CMVS
/// membership is decidable at the CLK (§ 7.3.7: the at-most-one MSDO precedes every
/// coded extended layer unit), so the cross-layer agreement check sees `Inside` when
/// the second CLK activates — the begin direction of the boundary that the two-TU
/// `cmvs_two_layer_stream` does not exercise.
pub(in crate::validator::tests) fn cmvs_two_layer_single_tu_stream(
    monotonic_x0: bool,
    monotonic_x1: bool,
) -> Vec<u8> {
    let mut data = temporal_delimiter_obu(); // single temporal unit
    data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO -> opens the CMVS
    data.extend(seq_header_obu_monotonic(0, 0, monotonic_x0)); // xlayer 0 seq 0
    data.extend(seq_header_obu_monotonic(1, 1, monotonic_x1)); // xlayer 1 seq 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, ref seq 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1, ref seq 1
    data
}

#[test]
fn monotonic_output_order_disagreement_in_cmvs_opening_tu_is_flagged() {
    let data = cmvs_two_layer_single_tu_stream(true, false);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "report was: {report}"
    );
}

#[test]
fn monotonic_output_order_agreement_in_cmvs_opening_tu_is_conforming() {
    let data = cmvs_two_layer_single_tu_stream(true, true);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "agreeing flags in the CMVS-opening temporal unit must not fire; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_agreement_inside_cmvs_is_conforming() {
    let data = cmvs_two_layer_stream(true, true);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "agreeing flags inside a CMVS must not fire; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_disagreement_outside_cmvs_is_not_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 monotonic 1
    data.extend(seq_header_obu_monotonic(1, 1, false)); // xlayer 1 monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // activate xlayer 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1)); // activate xlayer 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "disagreement outside any CMVS must not fire; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_disagreement_in_unknown_cmvs_is_not_flagged() {
    let mut data = temporal_delimiter_obu(); // temporal unit 1
    data.extend(global_lcr_obu(0, 0b11, None)); // global LCR (xlayers 0, 1), no MSDO
    data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 monotonic 1
    data.extend(seq_header_obu_monotonic(1, 1, false)); // xlayer 1 monotonic 0
    data.extend(annex_b_obu(0x10, &[])); // CLK xlayer 0 -> Unknown (LCR present, no MSDO)
    data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS now Unknown
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // re-activate xlayer 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "disagreement while the CMVS tracker is Unknown must not fire; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_disagreement_in_cmvs_ending_tu_is_not_flagged() {
    let mut data = temporal_delimiter_obu(); // temporal unit 1
    data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO -> begins the CMVS
    data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 seq 0 monotonic 1
    data.extend(annex_b_obu(0x04, &seq_header_payload_monotonic(1, false))); // seq 1 monotonic 0 (available)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, ref seq 0
    data.extend(temporal_delimiter_obu());
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1, ref seq 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "an MSDO-less CLK temporal unit ends the CMVS; a disagreement activated there \
         is outside the CMVS and must not fire; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_unreferenced_extra_header_inside_cmvs_is_not_flagged() {
    let mut data = temporal_delimiter_obu(); // temporal unit 1
    data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO
    data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 seq 0 monotonic 1
    data.extend(seq_header_obu_monotonic(1, 2, false)); // xlayer 1 extra, unreferenced
    data.extend(seq_header_obu_monotonic(1, 1, true)); // xlayer 1 referenced header
    data.extend(annex_b_obu(0x10, &[])); // CLK xlayer 0 -> begins the CMVS
    data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS now Inside
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // frame-confirm xlayer 0 (seq 0)
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1)); // frame-confirm xlayer 1 (seq 1)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "an unreferenced extra header with a differing flag must not fire (§ 7.3.6 \
         leaves it unactivated); report was: {report}"
    );
}

#[test]
fn monotonic_output_order_disagreement_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let data = cmvs_two_layer_stream(true, false);
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "external HLS declaring a sequence header must suppress the § 6.4.1 monotonic \
         agreement check; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_disagreement_under_empty_external_hls_is_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let data = cmvs_two_layer_stream(true, false);
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new()),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "an empty external set declares no sequence header and must not suppress the \
         § 6.4.1 monotonic agreement check; report was: {report}"
    );
}

/// Builds the first two temporal units shared by the
/// `monotonic_output_order_*_provisional_*` tests: TU1 opens a CMVS (MSDO + CLK,
/// begin condition 1) and frame-confirms xlayer 0 to seq 0 (`monotonic 1`); xlayer 1
/// carries seq 1 (`monotonic 1`). TU2 frame-confirms xlayer 1 to seq 1. Both layers
/// agree on `monotonic_output_order_flag == 1` and the CMVS is committed `Inside`
/// after TU2. The caller appends a TU3 whose shape exercises the provisional-Inside
/// deferral.
pub(in crate::validator::tests) fn cmvs_provisional_inside_prefix() -> Vec<u8> {
    let mut data = temporal_delimiter_obu(); // temporal unit 1
    data.extend(annex_b_obu(0x50, &msdo_payload(0))); // global MSDO -> opens the CMVS
    data.extend(seq_header_obu_monotonic(0, 0, true)); // xlayer 0 seq 0 monotonic 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(seq_header_obu_monotonic(1, 1, true)); // xlayer 1 seq 1 monotonic 1
    data.extend(temporal_delimiter_obu()); // temporal unit 2: CMVS committed Inside
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1));
    data
}

#[test]
fn monotonic_output_order_provisional_inside_clk_ending_tu_is_not_flagged() {
    let mut data = cmvs_provisional_inside_prefix();
    data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO)
    data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "a header redefinition at the top of a CMVS-ending CLK temporal unit is outside \
         the CMVS once the CLK is seen; the provisional header-time verdict must be \
         dropped; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_provisional_inside_mid_cmvs_redefinition_is_flagged() {
    let mut data = cmvs_provisional_inside_prefix();
    data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO, no CLK)
    data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "a mid-CMVS redefinition disagreeing on monotonic_output_order_flag must be \
         emitted at temporal-unit flush; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_provisional_inside_flushes_at_end_of_bitstream() {
    let mut data = cmvs_provisional_inside_prefix();
    data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO)
    data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "sequence-state/monotonic-output-order-mismatch"
                && d.spec_section.as_deref() == Some("6.4.1")
        }),
        "a disagreeing redefinition with no following CLK stays inside the CMVS and must \
         be emitted at the end-of-bitstream flush; report was: {report}"
    );
}

#[test]
fn monotonic_output_order_provisional_inside_unknown_clk_is_not_flagged() {
    let mut data = cmvs_provisional_inside_prefix();
    data.extend(temporal_delimiter_obu()); // temporal unit 3 (no MSDO)
    data.extend(global_lcr_obu(0, 0b11, None)); // global LCR (xlayers 0, 1), no MSDO
    data.extend(seq_header_obu_monotonic(0, 0, false)); // seq 0 redefined monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/monotonic-output-order-mismatch"),
        "a CLK temporal unit with a global LCR and no MSDO routes the tracker to \
         Unknown; the provisional verdict must be dropped; report was: {report}"
    );
}
