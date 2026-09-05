// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

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
    seq_header_payload_lcr_ref(
        seq_header_id,
        profile_idc,
        level_idx,
        tier_high,
        monotonic,
        0,
        0,
    )
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

#[test]
fn msdo_profile_below_substream_max_is_flagged() {
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

#[test]
fn msdo_reserved_multistream_profile_is_flagged() {
    let mut data = temporal_delimiter_obu();
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
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(31, true, &[(0, 4, 4, 0), (1, 4, 4, 0)]));
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

#[test]
fn doh_constraint_not_flagged_when_clk_ends_cmvs_for_the_activating_tu() {
    let mut data = temporal_delimiter_obu(); // temporal unit 1
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 21, 21, 0), (1, 21, 21, 0)],
    ));
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, true)); // seq 0 monotonic 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
    data.extend(temporal_delimiter_obu()); // temporal unit 2 (no MSDO)
    data.extend(seq_header_obu_ptl(0, 1, 0, 0, false, false)); // seq 1 monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 1)); // non-CLK frame activates seq 1
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
    let mut data = temporal_delimiter_obu(); // temporal unit 1 (no CMVS yet)
    data.extend(seq_header_obu_ptl(0, 0, 0, 0, false, false)); // seq 0 monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 0, 0)); // non-CLK frame confirms seq 0
    data.extend(temporal_delimiter_obu()); // temporal unit 2
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

/// A temporal-unit-delimited stream: each entry is `(make_rap, msdo)` where
/// `make_rap` adds a CLK (§ 7.4.1 random access point) to that temporal unit and
/// `msdo` is the MSDO payload bytes for that temporal unit (already a full OBU).
pub(in crate::validator::tests) fn msdo_identity_stream(units: &[(bool, Vec<u8>)]) -> Vec<u8> {
    let mut data = Vec::new();
    for (make_rap, msdo) in units {
        data.extend(temporal_delimiter_obu());
        data.extend_from_slice(msdo);
        if *make_rap {
            data.extend(annex_b_obu(0x10, &[])); // CLK on xlayer 0
        }
    }
    data
}

#[test]
fn non_rap_changed_msdo_is_flagged() {
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
    let msdo_a = msdo_obu_configured(2, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let msdo_b = msdo_obu_configured(3, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = msdo_identity_stream(&[(true, msdo_a), (false, msdo_b)]);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "msdo/non-rap-not-identical"),
        "the end-of-stream flush must resolve the final TU's MSDO; report was: {report}"
    );
}
