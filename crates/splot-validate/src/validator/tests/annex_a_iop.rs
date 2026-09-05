// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn annex_a_iop0_two_xlayers_without_msdo_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true)); // xlayer 0, profile 0 (IOP 0)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
    data.extend(seq_header_obu_ptl(1, 1, 0, 0, false, true)); // xlayer 1, profile 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/msdo-required-for-iop" && d.spec_section.as_deref() == Some("A.2")
        }),
        "a two-xlayer IOP0 CVS without an MSDO must be flagged; report was: {report}"
    );
}

#[test]
fn annex_a_iop0_single_xlayer_without_msdo_is_conforming() {
    let mut data = temporal_delimiter_obu();
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("annex-a/msdo-")
                || d.rule_id == "annex-a/lcr-required-for-iop"),
        "a single-xlayer IOP0 CVS needs no MSDO/LCR; report was: {report}"
    );
}

#[test]
fn annex_a_iop0_two_xlayers_with_msdo_is_conforming() {
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(seq_header_obu_ptl(1, 1, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("annex-a/msdo-")),
        "the MSDO satisfies the IOP0 multi-xlayer requirement; report was: {report}"
    );
}

#[test]
fn annex_a_iop_window_seeds_pre_clk_msdo_to_new_cvs() {
    let mut data = temporal_delimiter_obu(); // TU1: single-xlayer IOP0, no MSDO
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(temporal_delimiter_obu()); // TU2: MSDO precedes the CLK
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(seq_header_obu_ptl(0, 1, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 starts the new CVS
    data.extend(seq_header_obu_ptl(1, 2, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
        "TU2's two-xlayer CVS has its required MSDO; report was: {report}"
    );
}

#[test]
fn annex_a_iop2_requires_global_lcr_unactivated_does_not_satisfy() {
    let global = global_lcr_obu_agreement(1, 0b1, None, None, false);
    let mut data = temporal_delimiter_obu();
    data.extend(global);
    let payload = seq_header_payload_lcr_ref(0, 2, 0, false, true, 0, 1);
    data.extend(annex_b_obu(0x04, &payload));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/lcr-required-for-iop" && d.spec_section.as_deref() == Some("A.2")
        }),
        "an unactivated global LCR does not satisfy the IOP2 LCR requirement; report: {report}"
    );
}

#[test]
fn annex_a_iop2_requires_global_lcr_activated_satisfies() {
    let global = global_lcr_obu_agreement(1, 0b1, None, None, false);
    let mut data = temporal_delimiter_obu();
    data.extend(global);
    let payload = seq_header_payload_lcr_ref(0, 2, 0, false, true, 1, 1); // seq_lcr_id 1
    data.extend(annex_b_obu(0x04, &payload));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 1, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/lcr-required-for-iop"),
        "an activated global LCR satisfies the IOP2 LCR requirement; report was: {report}"
    );
}

#[test]
fn annex_a_iop_window_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(seq_header_obu_ptl(1, 1, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("annex-a/") && d.rule_id.ends_with("-for-iop")),
        "external HLS suppresses the Table A.4 presence checks; report was: {report}"
    );
}

#[test]
fn annex_a_iop_window_silent_for_reserved_profile() {
    let mut data = temporal_delimiter_obu();
    let payload = seq_header_payload_lcr_ref(0, 5, 0, false, true, 0, 0);
    data.extend(annex_b_obu(0x04, &payload));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    let payload1 = seq_header_payload_lcr_ref(1, 5, 0, false, true, 0, 0);
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(1, 0, 0, 1),
        &payload1,
    ));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id.ends_with("-for-iop")),
        "a reserved profile leaves the Table A.4 row undeterminable; report was: {report}"
    );
}

#[test]
fn annex_a_iop_same_id_reactivation_seeds_new_window() {
    let mut data = temporal_delimiter_obu(); // TU1: confirm xlayer 0 seq 0
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
    data.extend(temporal_delimiter_obu()); // TU2: a second xlayer + a same-id CLK
    data.extend(seq_header_obu_ptl(1, 1, 0, 0, false, true)); // xlayer 1 seq 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // same-id CLK xlayer 0, seq 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
        "a same-id CLK reactivation must seed the new IOP0 window so the two-xlayer CVS \
         without an MSDO is flagged; report was: {report}"
    );
}

#[test]
fn annex_a_iop_window_late_tu_second_xlayer_counts() {
    let mut data = temporal_delimiter_obu(); // TU1: opens the CVS with xlayer 0
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
    data.extend(temporal_delimiter_obu()); // TU2: a second xlayer joins (no CLK new-CVS)
    data.extend(seq_header_obu_ptl(1, 1, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1)); // non-CLK frame xlayer 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
        "a second xlayer in a later TU of the same CVS must reach E > 1; report was: {report}"
    );
}

#[test]
fn annex_a_iop_declared_count_precedence_over_observed() {
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // only xlayer 0 is coded
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
        "the present MSDO satisfies the multi-xlayer requirement; report was: {report}"
    );
}

#[test]
fn annex_a_iop_window_uses_association_time_global_lcr_snapshot() {
    let global_a = global_lcr_obu_agreement(1, 0b1, None, None, false); // count 1
    let global_b = global_lcr_obu_agreement(1, 0b11, None, None, false); // count 2 (redefine)
    let mut data = temporal_delimiter_obu(); // TU1: rev A present, header associates rev A
    data.extend(global_a);
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // profile 0, seq_lcr_id 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK confirms xlayer 0
    data.extend(temporal_delimiter_obu()); // TU2: rev B redefines id 1, then same-id CLK
    data.extend(global_b); // redefine id 1 AFTER the header associated rev A
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // same-id CLK re-activates
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
        "the IOP window must use the association-time rev A (LcrMaxNumXLayerCount 1, E == 1) \
         so the single-xlayer IOP0 CVS requires no MSDO; report was: {report}"
    );
}

pub(in crate::validator::tests) const MFH_HEADER: u8 = 3 << 2; // OBU_MULTI_FRAME_HEADER (0x0C)
pub(in crate::validator::tests) const BRT_HEADER: u8 = 15 << 2; // OBU_BUFFER_REMOVAL_TIMING (0x3C)
pub(in crate::validator::tests) const LEADING_SEF_HEADER: u8 = 11 << 2; // OBU_LEADING_SEF (0x2C)
pub(in crate::validator::tests) const REGULAR_SEF_HEADER: u8 = 12 << 2; // OBU_REGULAR_SEF (0x30)

#[test]
fn annex_a_iop0_more_than_one_embedded_layer_exceeds_budget() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &seq_header_payload_lcr_ref(0, 0, 0, false, true, 0, 1),
    ));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 confirms activation
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/layer-budget-exceeds-iop"
                && d.spec_section.as_deref() == Some("A.2")
        }),
        "an IOP0 CVS with two embedded layers exceeds the Table A.3 budget; report was: {report}"
    );
}

#[test]
fn annex_a_iop1_extended_and_embedded_combination_exceeds_budget() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &seq_header_payload_lcr_ref(0, 1, 0, false, true, 0, 1),
    ));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
    data.extend(seq_header_obu_ptl(1, 1, 1, 0, false, true)); // xlayer 1, profile 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1 (E)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/layer-budget-exceeds-iop"
                && d.spec_section.as_deref() == Some("A.2")
        }),
        "an IOP1 CVS with both >1 extended and >1 embedded layer exceeds the Table A.3 budget; \
         report was: {report}"
    );
}

#[test]
fn annex_a_iop0_single_embedded_layer_is_within_budget() {
    let mut data = temporal_delimiter_obu();
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true)); // profile 0, max_mlayer_id 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/layer-budget-exceeds-iop"),
        "a single-layer IOP0 CVS is within budget; report was: {report}"
    );
}

#[test]
fn annex_a_iop2_extended_and_embedded_combination_is_within_budget() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &seq_header_payload_lcr_ref(0, 2, 0, false, true, 0, 1),
    )); // xlayer 0: profile 2, max_mlayer_id 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(seq_header_obu_ptl(1, 1, 2, 0, false, true)); // xlayer 1: profile 2
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/layer-budget-exceeds-iop"),
        "IOP2 permits the Extended-and-Embedded combination; report was: {report}"
    );
}

#[test]
fn annex_a_layer_budget_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &seq_header_payload_lcr_ref(0, 0, 0, false, true, 0, 1),
    ));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/layer-budget-exceeds-iop"),
        "external HLS suppresses the Table A.3 layer-budget check; report was: {report}"
    );
}
