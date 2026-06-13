// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

// -- Annex A Table A.4 IOP presence re-land ---------------------------------

#[test]
fn annex_a_iop0_two_xlayers_without_msdo_is_flagged() {
    // Table A.4 row "0 Y": a profile-0 (IOP 0) coded video sequence with two distinct
    // non-global obu_xlayer_id values and no OBU_MSDO requires an OBU_MSDO. PR #46
    // scenario: multi-xlayer stream without MSDO. Both layers are frame-confirmed.
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
    // Table A.4 row "0 N": a single-extended-layer IOP0 CVS prohibits an MSDO and does
    // not require one — no diagnostic with one xlayer and no MSDO.
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
    // Table A.4 row "0 Y": with the required OBU_MSDO present, no diagnostic. The MSDO's
    // multistream_profile_idc 0 sets the IOP to 0.
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
    // PR #46 scenario: pre-CLK MSDO belongs to the new sequence. TU1 is a single-xlayer
    // IOP0 CVS (no MSDO, conforming). TU2 carries an OBU_MSDO BEFORE its CLK; § 7.3.6
    // attributes that MSDO to the NEW coded video sequence (TU2), not TU1. So TU1's
    // window has no MSDO (and one xlayer — conforming), and the prohibited-MSDO rule does
    // not fire against TU1. The window machinery must not have leaked TU2's pre-CLK MSDO
    // into TU1's evaluation.
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
    // TU1's window (one xlayer, no MSDO) is conforming; TU2's window (two xlayers, MSDO
    // present) is conforming. No prohibited-MSDO false positive against TU1.
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/msdo-prohibited-for-iop"),
        "the pre-CLK MSDO belongs to the new CVS, not TU1; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
        "TU2's two-xlayer CVS has its required MSDO; report was: {report}"
    );
}

#[test]
fn annex_a_iop2_requires_global_lcr_unactivated_does_not_satisfy() {
    // PR #46 scenario: an unactivated global LCR does not satisfy the arm. Table A.4 row
    // "2 N Y" (IOP2, one xlayer, two embedded layers): MSDO prohibited; a local or
    // activated global LCR required. A global LCR is present but NEVER activated
    // (seq_lcr_id == 0), so the requirement still fails.
    let global = global_lcr_obu_agreement(1, 0b1, None, None, false);
    let mut data = temporal_delimiter_obu();
    data.extend(global);
    // Single xlayer 0, profile 2 (IOP 2), with two embedded layers (max_mlayer_id 1),
    // referencing no LCR (seq_lcr_id 0), so the global LCR is never activated.
    let payload = seq_header_payload_lcr_ref(0, 2, 0, false, true, 0, 1);
    data.extend(annex_b_obu(0x04, &payload));
    // A CLK frame at obu_mlayer_id 0 confirms the activation, and a second frame at
    // obu_mlayer_id 1 makes a second embedded layer present.
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
    // Table A.4 row "2 N Y" boundary: the same IOP2 one-xlayer two-embedded-layer CVS,
    // but the header references the global LCR (seq_lcr_id == 1) so it is ACTIVATED — the
    // activated global LCR satisfies the requirement.
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
    // The Table A.4 presence checks need in-band HLS completeness, so they are suppressed
    // under any Provided external HLS — even the otherwise-flagged two-xlayer IOP0 CVS
    // without an MSDO.
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
    // A reserved seq_profile_idc (5) has no table-determined interoperability point, so
    // the Table A.4 row is not determinable and the presence check stays silent (the
    // reserved profile itself is flagged by annex-a/profile-reserved).
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
    // PR #46 scenario: same-id reactivation. A profile-0 (IOP0) header is frame-confirmed
    // in TU1; TU2 has a SECOND xlayer's header plus a CLK that re-references the SAME
    // already-active header for xlayer 0 (no id change, so on_sequence_activation is
    // skipped). The new CVS's IOP must be seeded from the active confirmed header so the
    // two-xlayer IOP0 CVS without an MSDO is still flagged.
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
    // PR #46 scenario: late-TU second xlayer. A second extended layer appears in a LATER
    // temporal unit of the SAME coded video sequence (no intervening CLK opens a new
    // CVS), so the window's distinct-xlayer count reaches 2 and the IOP0 multi-xlayer
    // MSDO requirement fires for the whole-CVS window.
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
    // PR #46 scenario: declared-count precedence (Table A.3 definition order). An MSDO
    // declares num_streams_minus_2 + 2 = 2 (E > 1) even though only ONE distinct
    // non-global obu_xlayer_id (0) is actually coded. The declared count takes precedence
    // (mirror lines 148-149), so E > 1 and the IOP0 multi-xlayer requirement is satisfied
    // by the present MSDO — and crucially the prohibited-MSDO rule (which needs E == 1)
    // does NOT fire against the single observed xlayer.
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // only xlayer 0 is coded
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/msdo-prohibited-for-iop"),
        "the MSDO's declared count (2) takes precedence over the single observed xlayer, \
         so the MSDO is not prohibited; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/msdo-required-for-iop"),
        "the present MSDO satisfies the multi-xlayer requirement; report was: {report}"
    );
}

#[test]
fn annex_a_iop_window_uses_association_time_global_lcr_snapshot() {
    // claude-review nit 4 (3393139837): the Table A.4 IOP window's activated-LCR
    // accounting must read `LcrMaxNumXLayerCount` from the *association-time* snapshot
    // (`LcrAssociation.global_record`), exactly like the § 6.8.2 agreement path, NOT a
    // live `global_lcr_records` lookup. A same-id global-LCR redefinition mid-CVS with a
    // different `lcr_xlayer_map` otherwise retargets the window's extended-layer count to
    // the later revision.
    //
    // Global LCR id 1 rev A has lcr_xlayer_map 0b1 -> LcrMaxNumXLayerCount 1 (E == 1).
    // Rev B redefines id 1 with lcr_xlayer_map 0b11 -> LcrMaxNumXLayerCount 2 (E > 1).
    // The header (profile 0 -> IOP0) associates rev A in TU1 and is frame-confirmed; that
    // window correctly counts E == 1. TU2 redefines id 1 to rev B, then a same-id CLK
    // re-references the still-active header, re-firing the IOP activation note that seeds
    // TU2's (new-CVS) window. The snapshot path keeps the activated count at rev A's 1
    // (E == 1, IOP0 -> MSDO neither required nor present -> no error). Pre-fix, the live
    // lookup at re-activation time sees rev B's count 2 (E > 1), so the IOP0 multi-xlayer
    // `annex-a/msdo-required-for-iop` fires falsely against a CVS with no MSDO.
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

// --- coded-frame-unit segmentation (AV2 § 7.3.3 / § 7.3.4 / § 7.3.5 / § 7.3.8.10) ---

// OBU header bytes (obu_type << 2, no extension, base layer):
pub(in crate::validator::tests) const MFH_HEADER: u8 = 3 << 2; // OBU_MULTI_FRAME_HEADER (0x0C)
pub(in crate::validator::tests) const BRT_HEADER: u8 = 15 << 2; // OBU_BUFFER_REMOVAL_TIMING (0x3C)
pub(in crate::validator::tests) const LEADING_SEF_HEADER: u8 = 11 << 2; // OBU_LEADING_SEF (0x2C)
pub(in crate::validator::tests) const REGULAR_SEF_HEADER: u8 = 12 << 2; // OBU_REGULAR_SEF (0x30)

// -- Annex A Table A.3 interoperability-point layer budget --------------------

#[test]
fn annex_a_iop0_more_than_one_embedded_layer_exceeds_budget() {
    // Table A.3 IOP0 (mirror line 130): Number of Embedded Layers must be 1. A profile-0
    // (IOP0) single-extended-layer CVS that declares max_mlayer_id 1 (two embedded layers)
    // exceeds the budget.
    let mut data = temporal_delimiter_obu();
    // profile 0, level 0, seq_lcr_id 0, max_mlayer_id 1 (-> SeqMaxMlayerCnt 2).
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
    // Table A.3 IOP1 (mirror line 132): the Extended-and-Embedded combination must be 0. A
    // profile-1 (IOP1) CVS with two extended layers (E) and two embedded layers (M) sets the
    // combination flag to 1, which IOP1 forbids. This is exactly the E && M case that has no
    // Table A.4 row, so the layer budget is its only constraint.
    let mut data = temporal_delimiter_obu();
    // xlayer 0: profile 1, max_mlayer_id 1 (M); xlayer 1: profile 1, max_mlayer_id 0.
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
    // A conformant IOP0 single-extended-single-embedded-layer CVS is within the Table A.3
    // budget — no layer-budget diagnostic.
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
    // Table A.3 IOP2 (mirror line 134): the Extended-and-Embedded combination MAY be 1. The
    // same E && M shape that IOP1 forbids is permitted at IOP2, so the layer-budget check
    // does not fire (other Table A.4 presence rules may, but those are a different rule_id).
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
    // The Annex A IOP window (including the Table A.3 budget) needs in-band HLS completeness,
    // so it is suppressed under any Provided external HLS.
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
