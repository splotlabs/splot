// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

// --- § 6.6 MSDO sub-stream constraints / § 7.3.8.2 identity (AV2-5.6-MSDO) ----

/// One sub-stream entry for [`msdo_obu_configured`]: `(sub_xlayer_id,
/// sub_stream_max_profile, sub_stream_max_level, sub_stream_max_tier)`.
pub(in crate::validator::tests) type SubStreamEntry = (u32, u32, u32, u32);

/// A global OBU_MSDO with the given `multistream_profile_idc`,
/// `multistream_doh_constraint_flag`, and per-substream entries (`num_streams_minus_2
/// = entries.len() - 2`). `multistream_level_idx` / `multistream_tier` are 0 and
/// allocation is even.
pub(in crate::validator::tests) fn msdo_obu_configured(
    multistream_profile_idc: u32,
    doh_constraint_flag: bool,
    entries: &[SubStreamEntry],
) -> Vec<u8> {
    assert!(entries.len() >= 2, "an MSDO has at least 2 sub-streams");
    let num_streams_minus_2 = (entries.len() - 2) as u32;
    let mut bits = Bits::default();
    bits.f(num_streams_minus_2, 3); // num_streams_minus_2
    bits.f(multistream_profile_idc, 5); // multistream_profile_idc
    bits.f(0, 5); // multistream_level_idx
    bits.bit(0); // multistream_tier
    bits.bit(1); // multistream_even_allocation_flag
    for &(sub_xlayer_id, max_profile, max_level, max_tier) in entries {
        bits.f(sub_xlayer_id, 5); // sub_xlayer_id
        bits.f(max_profile, 5); // sub_stream_max_profile
        bits.f(max_level, 5); // sub_stream_max_level
        bits.f(max_tier, 1); // sub_stream_max_tier
    }
    bits.bit(u8::from(doh_constraint_flag)); // multistream_doh_constraint_flag
    bits.bit(1); // trailing_one_bit (valid trailing_bits)
    annex_b_obu(0x50, &bits.into_bytes())
}

/// A sequence-header payload with explicit `seq_profile_idc`, `seq_level_idx`,
/// `seq_tier`, and `monotonic_output_order_flag`, `max_*layer_id == 0`. `seq_tier` is
/// only signaled when `seq_level_idx > 3` (§ 5.4.1); the caller must pick a level
/// above 3 to exercise a High tier. The payload is a complete, activatable header.
pub(in crate::validator::tests) fn seq_header_payload_ptl(
    seq_header_id: u32,
    profile_idc: u32,
    level_idx: u32,
    tier_high: bool,
    monotonic: bool,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(seq_header_id);
    bits.f(profile_idc, 5); // seq_profile_idc
    bits.bit(0); // single_picture_header_flag
    bits.f(level_idx, 5); // seq_level_idx
    if level_idx > 3 {
        bits.bit(u8::from(tier_high)); // seq_tier (signaled only for level > 3)
    }
    bits.uvlc(0); // chroma_format_idc
    bits.uvlc(0); // bit_depth_idc
    bits.f(0, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(0, 2); // max_tlayer_id = 0
    bits.f(0, 3); // max_mlayer_id = 0
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

/// A sequence-header OBU on `xlayer` carrying [`seq_header_payload_ptl`].
pub(in crate::validator::tests) fn seq_header_obu_ptl(
    xlayer: u8,
    seq_header_id: u32,
    profile_idc: u32,
    level_idx: u32,
    tier_high: bool,
    monotonic: bool,
) -> Vec<u8> {
    let payload =
        seq_header_payload_ptl(seq_header_id, profile_idc, level_idx, tier_high, monotonic);
    if xlayer == 0 {
        annex_b_obu(0x04, &payload)
    } else {
        annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
    }
}

// -- Task 2.1: msdo/profile-below-substream-max (locally decidable) -----------

#[test]
fn msdo_profile_below_substream_max_is_flagged() {
    // § 6.6: multistream_profile_idc (1) < sub_stream_max_profile[1] (3) — flagged.
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(1, true, &[(0, 0, 0, 0), (1, 3, 0, 0)]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "msdo/profile-below-substream-max"
                && d.spec_section.as_deref() == Some("6.6")
        }),
        "report was: {report}"
    );
}

#[test]
fn msdo_profile_equal_to_substream_max_is_conforming() {
    // § 6.6 boundary: multistream_profile_idc (3) == sub_stream_max_profile (3) — ok.
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(3, true, &[(0, 3, 0, 0), (1, 2, 0, 0)]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "msdo/profile-below-substream-max"),
        "equality must pass; report was: {report}"
    );
}

// -- Task 2.2: annex-a/profile-reserved for multistream_profile_idc ----------

#[test]
fn msdo_reserved_multistream_profile_is_flagged() {
    // § 6.6: multistream_profile_idc 7 is reserved (5..=30) — annex-a/profile-reserved.
    let mut data = temporal_delimiter_obu();
    // sub_stream_max_profile must be <= multistream_profile_idc to isolate the
    // reserved-profile finding from the floor check.
    data.extend(msdo_obu_configured(7, true, &[(0, 7, 0, 0), (1, 7, 0, 0)]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/profile-reserved" && d.message.contains("multistream_profile_idc")
        }),
        "report was: {report}"
    );
}

#[test]
fn msdo_valid_multistream_profile_is_not_reserved() {
    // multistream_profile_idc 4 is a defined profile — no profile-reserved error.
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(4, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/profile-reserved"),
        "multistream_profile_idc 4 is defined; report was: {report}"
    );
}

// -- Task 3.2: sub-stream PTL-ceiling agreement, both arrival orders ---------

/// Builds a single-temporal-unit two-layer multistream stream that opens a CMVS
/// (MSDO + CLK), with sequence headers carrying the given PTL, then frame-confirms
/// each extended layer via a CLK frame referencing its header. `msdo_first` controls
/// the arrival order: when true the MSDO precedes the headers/activations
/// (MSDO-then-activation); when false it follows the activations
/// (activation-then-MSDO). The MSDO declares sub_xlayer_id 0 and 1 with the given
/// ceilings and a satisfied DOH flag.
pub(in crate::validator::tests) fn substream_ptl_stream(
    msdo_first: bool,
    seq0: (u32, u32, bool),
    seq1: (u32, u32, bool),
    ceil0: (u32, u32, u32),
    ceil1: (u32, u32, u32),
) -> Vec<u8> {
    let msdo = msdo_obu_configured(
        31, // Configurable profile: a high floor so the profile-below check never fires
        true,
        &[
            (0, ceil0.0, ceil0.1, ceil0.2),
            (1, ceil1.0, ceil1.1, ceil1.2),
        ],
    );
    let headers_and_frames = {
        let mut d = Vec::new();
        d.extend(seq_header_obu_ptl(0, 0, seq0.0, seq0.1, seq0.2, true));
        d.extend(seq_header_obu_ptl(1, 1, seq1.0, seq1.1, seq1.2, true));
        // CLK frame headers confirm activation and (the first) opens the CMVS.
        d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
        d
    };
    let mut data = temporal_delimiter_obu();
    if msdo_first {
        data.extend(msdo);
        data.extend(headers_and_frames);
    } else {
        data.extend(headers_and_frames);
        data.extend(msdo);
    }
    data
}

#[test]
fn substream_level_exceeds_max_is_flagged_msdo_first() {
    // Spec scenario: MSDO sub_stream_max_level[1] = 4 for sub_xlayer_id 1; a
    // frame-confirmed header with seq_level_idx = 8 activates on extended layer 1.
    // MSDO arrives before the activations.
    let data = substream_ptl_stream(
        true,
        (0, 4, false), // xlayer 0 header: level 4
        (0, 8, false), // xlayer 1 header: level 8 -> exceeds ceiling 4
        (0, 21, 0),    // ceiling for xlayer 0
        (0, 4, 0),     // ceiling for xlayer 1: max_level 4
    );
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "msdo/substream-level-exceeds-max"
                && d.spec_section.as_deref() == Some("6.6")
        }),
        "report was: {report}"
    );
}

#[test]
fn substream_level_exceeds_max_is_flagged_activation_first() {
    // Same violation, MSDO arriving AFTER both activations (activation-then-MSDO).
    let data = substream_ptl_stream(false, (0, 4, false), (0, 8, false), (0, 21, 0), (0, 4, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "msdo/substream-level-exceeds-max"),
        "the violation must fire when the MSDO follows the activation; report was: {report}"
    );
}

#[test]
fn substream_level_equal_to_max_is_conforming() {
    // § 6.6 boundary: seq_level_idx (4) == sub_stream_max_level (4) — no diagnostic.
    let data = substream_ptl_stream(true, (0, 4, false), (0, 4, false), (0, 4, 0), (0, 4, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("msdo/substream-")),
        "equality must pass; report was: {report}"
    );
}

#[test]
fn substream_profile_exceeds_max_is_flagged() {
    // § 6.6: seq_profile_idc (4) on xlayer 1 exceeds sub_stream_max_profile (2).
    let data = substream_ptl_stream(true, (0, 0, false), (4, 0, false), (4, 21, 0), (2, 21, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "msdo/substream-profile-exceeds-max"),
        "report was: {report}"
    );
}

#[test]
fn substream_tier_exceeds_max_is_flagged() {
    // § 6.6: seq_tier High (1) on xlayer 1 (level 8 > 3 so tier is signaled) exceeds
    // sub_stream_max_tier (0).
    let data = substream_ptl_stream(
        true,
        (0, 8, false),
        (0, 8, true), // xlayer 1: High tier
        (0, 21, 1),
        (0, 21, 0), // ceiling tier 0
    );
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "msdo/substream-tier-exceeds-max"),
        "report was: {report}"
    );
}

#[test]
fn substream_max_not_flagged_for_unconfirmed_activation() {
    // Frame-confirmed gating: an OBU-order fallback header that no frame references
    // must NOT be checked against the MSDO ceiling (§ 7.3.6 staged-but-unactivated).
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(31, true, &[(0, 4, 4, 0), (1, 4, 4, 0)]));
    // Two staged headers on xlayer 1, neither frame-confirmed; the second has a level
    // above the ceiling. With two in-band candidates and no frame, neither is the
    // decidable sole-candidate activation.
    data.extend(seq_header_obu_ptl(1, 0, 0, 4, false, true));
    data.extend(seq_header_obu_ptl(1, 1, 0, 8, false, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("msdo/substream-")),
        "an unconfirmed staged header must not be checked; report was: {report}"
    );
}

#[test]
fn substream_max_suppressed_under_external_hls() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // The substream-max agreement is suppressed when external HLS declares a
    // sequence header (the activated header may be out-of-band with unmodeled PTL).
    let data = substream_ptl_stream(true, (0, 4, false), (0, 8, false), (0, 21, 0), (0, 4, 0));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(
            ExternalHlsSet::new()
                .with_sequence_header_id(0)
                .with_sequence_header_id(1),
        ),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("msdo/substream-")),
        "external HLS suppresses the substream-max agreement; report was: {report}"
    );
}

// -- Task 4: msdo/doh-constraint-required, both arrival orders ---------------

/// A single-temporal-unit two-layer multistream stream opening a CMVS, with the
/// extended-layer-0 header carrying `monotonic_x0` and extended-layer-1 header
/// carrying `monotonic_x1`, an MSDO with the given `doh_constraint_flag`, and
/// frame-confirmed activations. `msdo_first` selects the arrival order.
pub(in crate::validator::tests) fn doh_stream(
    msdo_first: bool,
    doh_flag: bool,
    monotonic_x0: bool,
    monotonic_x1: bool,
) -> Vec<u8> {
    let msdo = msdo_obu_configured(31, doh_flag, &[(0, 21, 21, 0), (1, 21, 21, 0)]);
    let headers_and_frames = {
        let mut d = Vec::new();
        d.extend(seq_header_obu_ptl(0, 0, 0, 0, false, monotonic_x0));
        d.extend(seq_header_obu_ptl(1, 1, 0, 0, false, monotonic_x1));
        d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
        d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
        d
    };
    let mut data = temporal_delimiter_obu();
    if msdo_first {
        data.extend(msdo);
        data.extend(headers_and_frames);
    } else {
        data.extend(headers_and_frames);
        data.extend(msdo);
    }
    data
}

#[test]
fn doh_constraint_required_is_flagged_msdo_first() {
    // § 6.6: a CMVS-inside activated header with monotonic_output_order_flag == 0
    // while multistream_doh_constraint_flag == 0 — flagged. MSDO arrives first.
    let data = doh_stream(true, false, true, false);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "msdo/doh-constraint-required" && d.spec_section.as_deref() == Some("6.6")
        }),
        "report was: {report}"
    );
}

#[test]
fn doh_constraint_required_is_flagged_activation_first() {
    // Same violation, MSDO arriving after the activations (activation-then-MSDO).
    let data = doh_stream(false, false, true, false);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "msdo/doh-constraint-required"),
        "the DOH requirement must fire when the MSDO follows the activation; report was: {report}"
    );
}

#[test]
fn doh_constraint_satisfied_by_flag_is_conforming() {
    // multistream_doh_constraint_flag == 1 satisfies the requirement even with a
    // non-monotonic activated header.
    let data = doh_stream(true, true, true, false);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "msdo/doh-constraint-required"),
        "doh_constraint_flag == 1 satisfies the requirement; report was: {report}"
    );
}

#[test]
fn doh_constraint_not_flagged_when_all_monotonic() {
    // Every activated header is monotonic (flag == 1), so the requirement is vacuous
    // even with multistream_doh_constraint_flag == 0.
    let data = doh_stream(true, false, true, true);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "msdo/doh-constraint-required"),
        "all-monotonic headers do not trigger the DOH requirement; report was: {report}"
    );
}

#[test]
fn doh_constraint_not_flagged_outside_cmvs() {
    // With no MSDO opening a CMVS the tracker stays Outside, so a non-monotonic
    // header with no DOH context does not fire. (Here there is no MSDO at all.)
    let mut data = temporal_delimiter_obu();
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, false)); // non-monotonic
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "msdo/doh-constraint-required"),
        "no CMVS means no DOH requirement; report was: {report}"
    );
}

// -- Codex PR #47 follow-ups: deferred DOH evaluation + duplicate ceilings ---

#[test]
fn doh_constraint_not_flagged_when_clk_ends_cmvs_for_the_activating_tu() {
    // Codex finding 3392940061. The DOH check must defer until the temporal unit's
    // CMVS membership is final. Scenario: TU1 opens a CMVS (MSDO with
    // multistream_doh_constraint_flag == 0, a monotonic-1 header, a CLK), so it is
    // Inside; TU2 redefines the active header to monotonic_output_order_flag == 0 and
    // activates it via a non-CLK frame BEFORE a later MSDO-less CLK ends the CMVS
    // (§ 7.3.2 end condition 2, mirror `07-decoding-process.md` lines 335-341). The
    // monotonic-0 header therefore sits OUTSIDE the CMVS, so § 6.6 does not apply to
    // it and no `msdo/doh-constraint-required` may fire. The pre-fix eager check,
    // gated on the still-`Inside` committed state at activation time (the ending CLK
    // is observed only later in TU2), fired a false positive.
    let mut data = temporal_delimiter_obu(); // temporal unit 1
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 21, 21, 0), (1, 21, 21, 0)],
    ));
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true)); // seq 0 monotonic 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
    data.extend(temporal_delimiter_obu()); // temporal unit 2 (no MSDO)
    // Redefinition: a new header (seq 1) for xlayer 0 with monotonic 0, activated by a
    // non-CLK frame so on_sequence_activation re-runs (the eager path the finding
    // describes). The ending CLK has not yet been observed when this activates.
    data.extend(seq_header_obu_ptl(0, 1, 0, 0, false, false)); // seq 1 monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1)); // non-CLK frame activates seq 1
    // A later MSDO-less CLK ends the CMVS for temporal unit 2 (end condition 2), so
    // the monotonic-0 header above is outside the CMVS.
    data.extend(annex_b_obu(0x10, &[])); // bare CLK on xlayer 0, no MSDO
    data.extend(temporal_delimiter_obu()); // close temporal unit 2 via a boundary
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "msdo/doh-constraint-required"),
        "a monotonic-0 header in a temporal unit whose MSDO-less CLK ends the CMVS is \
         outside the CMVS; § 6.6 must not fire; report was: {report}"
    );
}

#[test]
fn doh_constraint_flagged_when_same_id_clk_opens_cmvs_after_the_activation() {
    // Codex finding 3392940072. A header frame-confirmed BEFORE any CMVS, then a
    // temporal unit with an MSDO (multistream_doh_constraint_flag == 0) followed by a
    // same-id CLK that opens the CMVS. The same-id CLK re-references the already-active
    // header, so on_sequence_activation is skipped (the seq id is unchanged and the
    // layer was already frame-confirmed) — the pre-fix eager check, which only ran on
    // a (re)activation, never saw the CMVS transition to Inside and missed the
    // violation. The header has monotonic_output_order_flag == 0, so § 6.6 requires
    // multistream_doh_constraint_flag == 1; the deferred evaluation at temporal-unit
    // completion re-examines all frame-confirmed activations and fires.
    let mut data = temporal_delimiter_obu(); // temporal unit 1 (no CMVS yet)
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, false)); // seq 0 monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // non-CLK frame confirms seq 0
    data.extend(temporal_delimiter_obu()); // temporal unit 2
    // The MSDO (doh flag 0) precedes the coded extended layer unit (§ 7.3.7), and the
    // same-id CLK frame re-references seq 0 and opens the CMVS at that CLK.
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 21, 21, 0), (1, 21, 21, 0)],
    ));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, ref seq 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "msdo/doh-constraint-required" && d.spec_section.as_deref() == Some("6.6")
        }),
        "a same-id CLK that opens a CMVS over a monotonic-0 frame-confirmed header must \
         fire § 6.6 at temporal-unit resolution; report was: {report}"
    );
}

#[test]
fn substream_max_duplicate_sub_xlayer_id_keeps_the_most_restrictive_ceiling() {
    // Codex finding 3392940071. § 6.6 imposes the sub_stream_max_* ceiling "for each
    // sequence header activated by the i-th independent sub-stream" — for EACH i. With
    // a duplicate sub_xlayer_id (the spec declares no uniqueness requirement), an
    // activated header must satisfy BOTH declared ceilings, so the effective per-layer
    // ceiling is the per-dimension minimum. Here sub_xlayer_id 1 is declared twice with
    // sub_stream_max_level 8 and 4; a header at level 6 on extended layer 1 exceeds the
    // tighter ceiling 4 and must be flagged. A pre-fix last-wins insert would keep
    // whichever entry came last and miss the violation when the 8-ceiling won.
    for (first, second) in [
        ((1, 21, 8, 0), (1, 21, 4, 0)),
        ((1, 21, 4, 0), (1, 21, 8, 0)),
    ] {
        let mut data = temporal_delimiter_obu();
        data.extend(msdo_obu_configured(
            31,
            true,
            &[(0, 21, 21, 0), first, second],
        ));
        // Interleave each layer's header with its CLK frame in ascending xlayer order
        // (§ 7.3.7 coded-extended-layer-unit ordering): xlayer 0 header + frame, then
        // xlayer 1 header + frame.
        data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true)); // xlayer 0 header
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // confirm xlayer 0
        data.extend(seq_header_obu_ptl(1, 1, 0, 6, false, true)); // xlayer 1: level 6
        data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // confirm xlayer 1
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "msdo/substream-level-exceeds-max"
                    && d.spec_section.as_deref() == Some("6.6")
            }),
            "a duplicate sub_xlayer_id must enforce the most restrictive (level 4) \
             ceiling regardless of declaration order ({first:?} then {second:?}); \
             report was: {report}"
        );
    }
}

// -- Task 5: § 7.3.8.2 non-RAP MSDO identity --------------------------------

/// A temporal-unit-delimited stream: each entry is `(make_rap, msdo)` where
/// `make_rap` adds a CLK (§ 7.4.1 random access point) to that temporal unit and
/// `msdo` is the MSDO payload bytes for that temporal unit (already a full OBU).
pub(in crate::validator::tests) fn msdo_identity_stream(units: &[(bool, Vec<u8>)]) -> Vec<u8> {
    let mut data = Vec::new();
    for (make_rap, msdo) in units {
        data.extend(temporal_delimiter_obu());
        data.extend(msdo.clone());
        if *make_rap {
            data.extend(annex_b_obu(0x10, &[])); // CLK on xlayer 0
        }
    }
    data
}

#[test]
fn non_rap_changed_msdo_is_flagged() {
    // § 7.3.8.2: a non-RAP temporal unit carrying a changed OBU_MSDO — flagged.
    // A trailing temporal delimiter ends the offending TU, so the finding is emitted
    // from the TD-driven `complete_temporal_unit` path (distinct from the
    // end-of-stream flush exercised by the sibling test below).
    let msdo_a = msdo_obu_configured(2, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let msdo_b = msdo_obu_configured(3, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let mut data = msdo_identity_stream(&[
        (true, msdo_a),  // RAP TU establishes the reference
        (false, msdo_b), // non-RAP TU with a changed MSDO -> flagged
    ]);
    data.extend(temporal_delimiter_obu()); // end the offending TU via a TD boundary
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "msdo/non-rap-not-identical"
                && d.spec_section.as_deref() == Some("7.3.8.2")
        }),
        "report was: {report}"
    );
}

#[test]
fn non_rap_identical_msdo_is_conforming() {
    // § 7.3.8.2: a non-RAP temporal unit carrying an identical OBU_MSDO — no error.
    let msdo = msdo_obu_configured(2, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = msdo_identity_stream(&[(true, msdo.clone()), (false, msdo)]);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "msdo/non-rap-not-identical"),
        "an identical MSDO must pass; report was: {report}"
    );
}

#[test]
fn rap_changed_msdo_is_conforming() {
    // § 7.3.8.2: a RAP temporal unit (contains a CLK) carrying a changed OBU_MSDO is
    // exempt — no identity error, and the reference updates.
    let msdo_a = msdo_obu_configured(2, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let msdo_b = msdo_obu_configured(3, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = msdo_identity_stream(&[(true, msdo_a), (true, msdo_b)]);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "msdo/non-rap-not-identical"),
        "a changed MSDO at a random access point is exempt; report was: {report}"
    );
}

#[test]
fn non_rap_changed_msdo_at_end_of_stream_is_flagged() {
    // The final temporal unit has no trailing temporal delimiter; the end-of-stream
    // flush still resolves its buffered MSDO against the previous one.
    let msdo_a = msdo_obu_configured(2, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let msdo_b = msdo_obu_configured(3, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    // First TU is a RAP establishing the reference; second TU is the final,
    // non-RAP TU with a changed MSDO and no trailing delimiter.
    let data = msdo_identity_stream(&[(true, msdo_a), (false, msdo_b)]);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "msdo/non-rap-not-identical"),
        "the end-of-stream flush must resolve the final TU's MSDO; report was: {report}"
    );
}
