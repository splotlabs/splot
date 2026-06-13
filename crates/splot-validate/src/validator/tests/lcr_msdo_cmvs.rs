// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

// == msdo-global-lcr-agreement: § 6.8.2 / § 7.3.2 / Annex A Table A.4 =============

/// `lcr_aggregate_info()` fields for the configurable global-LCR builder.
#[derive(Clone, Copy)]
pub(in crate::validator::tests) struct AggInfo {
    pub(in crate::validator::tests) config_idc: u32,
    pub(in crate::validator::tests) aggregate_level_idx: u32,
    pub(in crate::validator::tests) max_tier_flag: u8,
    pub(in crate::validator::tests) max_interop: u32,
}

/// One `lcr_seq_profile_tier_level_info(i)` entry for the global-LCR builder, in the
/// xlayer-ascending order the parser reads them.
#[derive(Clone, Copy)]
pub(in crate::validator::tests) struct GlobalPtl {
    pub(in crate::validator::tests) seq_profile_idc: u32,
    pub(in crate::validator::tests) max_level_idx: u32,
    pub(in crate::validator::tests) tier_flag: u8,
    pub(in crate::validator::tests) max_mlayer_count: u32,
}

/// A global LCR OBU with the § 6.8.2 agreement fields configurable: the xlayer map, an
/// optional `lcr_aggregate_info()`, an optional per-xlayer `lcr_seq_profile_tier_level_info`
/// list (ascending xlayer order, one per set bit of `xlayer_map`), and the
/// `lcr_doh_constraint_flag`. No global payload.
pub(in crate::validator::tests) fn global_lcr_obu_agreement(
    global_id: u32,
    xlayer_map: u32,
    agg: Option<AggInfo>,
    ptls: Option<&[GlobalPtl]>,
    doh_constraint_flag: bool,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(global_id, 3); // lcr_global_config_record_id
    bits.f(xlayer_map, 31); // lcr_xlayer_map
    bits.bit(u8::from(agg.is_some())); // lcr_aggregate_info_present_flag
    bits.bit(u8::from(ptls.is_some())); // lcr_seq_profile_tier_level_info_present_flag
    bits.bit(0); // lcr_global_payload_present_flag
    bits.bit(0); // lcr_dependent_xlayers_flag
    bits.bit(0); // lcr_global_atlas_id_present_flag
    bits.f(0, 7); // lcr_global_purpose_id
    bits.bit(u8::from(doh_constraint_flag)); // lcr_doh_constraint_flag
    bits.bit(0); // lcr_enforce_tile_alignment_flag
    bits.f(0, 3); // lcr_global_reserved_zero_3bits
    bits.f(0, 5); // lcr_global_reserved_zero_5bits
    if let Some(agg) = agg {
        bits.f(agg.config_idc, 6); // lcr_config_idc
        bits.f(agg.aggregate_level_idx, 5); // lcr_aggregate_level_idx
        bits.bit(agg.max_tier_flag); // lcr_max_tier_flag
        bits.f(agg.max_interop, 4); // lcr_max_interop
    }
    if let Some(ptls) = ptls {
        for ptl in ptls {
            bits.f(ptl.seq_profile_idc, 5); // lcr_seq_profile_idc[i]
            bits.f(ptl.max_level_idx, 5); // lcr_max_level_idx[i]
            bits.bit(ptl.tier_flag); // lcr_tier_flag[i]
            bits.f(ptl.max_mlayer_count, 3); // lcr_max_mlayer_count[i]
            bits.f(0, 2); // lsptli_reserved_2bits
        }
    }
    extensible_obu_tail(&mut bits);
    annex_b_obu_with_header(&layer_obu_header(16, 0, 0, 31), &bits.into_bytes())
}

/// A sequence-header payload that references `seq_lcr_id` (so an activated frame for
/// this layer associates the header with that LCR), with explicit PTL and
/// `monotonic_output_order_flag`, `max_*layer_id == 0`.
pub(in crate::validator::tests) fn seq_header_payload_lcr_ref(
    seq_header_id: u32,
    profile_idc: u32,
    level_idx: u32,
    tier_high: bool,
    monotonic: bool,
    seq_lcr_id: u32,
    max_mlayer_id: u32,
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
    bits.f(seq_lcr_id, 3); // seq_lcr_id
    bits.bit(0); // still_picture
    bits.f(0, 2); // max_tlayer_id = 0
    bits.f(max_mlayer_id, 3); // max_mlayer_id
    if max_mlayer_id > 0 {
        // SeqMaxMlayerCnt = max_mlayer_id + 1 allows embedded layers 0..=max_mlayer_id.
        bits.f(max_mlayer_id, ceil_log2_u32(max_mlayer_id + 1)); // seq_max_mlayer_cnt_minus_1
    }
    bits.bit(u8::from(monotonic)); // monotonic_output_order_flag
    bits.f(3, 4); // frame_width_bits_minus_1
    bits.f(3, 4); // frame_height_bits_minus_1
    bits.f(15, 4); // max_frame_width_minus_1
    bits.f(7, 4); // max_frame_height_minus_1
    bits.bit(0); // seq_cropping_window_present_flag
    bits.bit(0); // seq_initial_display_delay_present_flag
    bits.bit(0); // decoder_model_info_present_flag
    if max_mlayer_id > 0 {
        bits.bit(0); // mlayer_dependency_present_flag
    }
    append_non_single_child_configs(&mut bits);
    bits.into_bytes()
}

/// A sequence-header OBU on `xlayer` carrying [`seq_header_payload_lcr_ref`].
pub(in crate::validator::tests) fn seq_header_obu_lcr_ref(
    xlayer: u8,
    seq_header_id: u32,
    profile_idc: u32,
    monotonic: bool,
    seq_lcr_id: u32,
) -> Vec<u8> {
    let payload = seq_header_payload_lcr_ref(
        seq_header_id,
        profile_idc,
        0,
        false,
        monotonic,
        seq_lcr_id,
        0,
    );
    if xlayer == 0 {
        annex_b_obu(0x04, &payload)
    } else {
        annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
    }
}

/// A two-extended-layer CMVS stream opening a CMVS in a single temporal unit (begin
/// condition 1: a CLK temporal unit with an MSDO present), with a global LCR present and
/// activated by both layers' headers via `seq_lcr_id == global_id`. `msdo_first` selects
/// the arrival order of the MSDO relative to the headers/global-LCR. Both layers are
/// frame-confirmed by CLK frames in the opening temporal unit.
#[allow(clippy::too_many_arguments)]
pub(in crate::validator::tests) fn lcr_msdo_stream(
    msdo_first: bool,
    global_id: u32,
    global_xlayer_map: u32,
    agg: Option<AggInfo>,
    ptls: Option<&[GlobalPtl]>,
    global_doh: bool,
    msdo: Vec<u8>,
) -> Vec<u8> {
    let global = global_lcr_obu_agreement(global_id, global_xlayer_map, agg, ptls, global_doh);
    let headers_and_frames = {
        // Global HLS (the global LCR) first, then per-layer coded extended layer units in
        // ascending obu_xlayer_id order (§ 7.3.7): seq0 + CLK0, then seq1 + CLK1.
        let mut d = Vec::new();
        d.extend(global.clone());
        d.extend(seq_header_obu_lcr_ref(0, 0, 0, true, global_id));
        d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
        d.extend(seq_header_obu_lcr_ref(1, 1, 0, true, global_id));
        d.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
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
fn lcr_msdo_stream_count_mismatch_is_flagged_both_orders() {
    // § 6.8.2 constraint 1: num_streams_minus_2 + 2 (2) != LcrMaxNumXLayerCount (3,
    // from a 3-bit xlayer_map 0b111). Flagged in both arrival orders.
    for msdo_first in [true, false] {
        let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
        let data = lcr_msdo_stream(msdo_first, 1, 0b111, None, None, false, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/msdo-stream-count-mismatch"
                    && d.spec_section.as_deref() == Some("6.8.2")
            }),
            "stream-count mismatch must fire (msdo_first={msdo_first}); report was: {report}"
        );
    }
}

#[test]
fn lcr_msdo_stream_count_match_is_conforming() {
    // § 6.8.2 constraint 1 boundary: num_streams (2) == LcrMaxNumXLayerCount (2, map
    // 0b11). No stream-count mismatch.
    let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = lcr_msdo_stream(true, 1, 0b11, None, None, false, msdo);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/msdo-stream-count-mismatch"),
        "matching stream count must not fire; report was: {report}"
    );
}

#[test]
fn lcr_msdo_sub_xlayer_not_in_lcr_is_flagged() {
    // § 6.8.2 constraint 2: an MSDO sub_xlayer_id (2) not in LcrXLayerID[] (the map
    // 0b11 sets bits 0 and 1 only). LcrMaxNumXLayerCount is 2 == num_streams, so only
    // the membership constraint fires.
    let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (2, 0, 0, 0)]);
    let data = lcr_msdo_stream(true, 1, 0b11, None, None, false, msdo);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/msdo-sub-xlayer-not-in-lcr"
                && d.spec_section.as_deref() == Some("6.8.2")
        }),
        "a sub_xlayer_id outside LcrXLayerID[] must fire; report was: {report}"
    );
}

#[test]
fn lcr_msdo_sub_xlayer_in_lcr_is_conforming() {
    // § 6.8.2 constraint 2 boundary: every sub_xlayer_id (0, 1) is in LcrXLayerID[]
    // (map 0b11). No membership mismatch.
    let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = lcr_msdo_stream(true, 1, 0b11, None, None, false, msdo);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/msdo-sub-xlayer-not-in-lcr"),
        "in-set sub_xlayer_ids must not fire; report was: {report}"
    );
}

#[test]
fn lcr_msdo_aggregate_level_and_tier_mismatch_is_flagged() {
    // § 6.8.2 constraint 3: multistream_level_idx (the msdo builder hardcodes 0) !=
    // lcr_aggregate_level_idx (5), and multistream_tier (0) != lcr_max_tier_flag (1).
    // multistream_profile_idc 0 -> config 0 allows it and IOP 0 == max_interop 0, so
    // only the level and tier arms fire.
    let agg = AggInfo {
        config_idc: 0,
        aggregate_level_idx: 5,
        max_tier_flag: 1,
        max_interop: 0,
    };
    let msdo = msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = lcr_msdo_stream(true, 1, 0b11, Some(agg), None, false, msdo);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/msdo-aggregate-mismatch"
                && d.spec_section.as_deref() == Some("6.8.2")
                && d.message.contains("multistream_level_idx")
        }),
        "an aggregate level mismatch must fire; report was: {report}"
    );
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/msdo-aggregate-mismatch" && d.message.contains("multistream_tier")
        }),
        "an aggregate tier mismatch must fire; report was: {report}"
    );
}

#[test]
fn lcr_msdo_aggregate_interop_and_config_mismatch_is_flagged() {
    // § 6.8.2 constraint 3: multistream_profile_idc 4 (IOP 1, and config 0 C_Main_420_10
    // does NOT allow profile 4 per Table A.6) vs lcr_config_idc 0 and lcr_max_interop 0.
    // So both the Table A.6 config consistency and the Table A.1 interop equality fire.
    let agg = AggInfo {
        config_idc: 0,
        aggregate_level_idx: 0,
        max_tier_flag: 0,
        max_interop: 0,
    };
    // multistream_profile_idc 4 needs a level > 3 in the headers to be conformant for
    // High tier, but profile alone is fine; msdo level is 0 here (matches agg level 0).
    let msdo = msdo_obu_configured(4, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = lcr_msdo_stream(true, 1, 0b11, Some(agg), None, false, msdo);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/msdo-aggregate-mismatch" && d.message.contains("lcr_config_idc")
        }),
        "a Table A.6 config-idc inconsistency must fire; report was: {report}"
    );
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/msdo-aggregate-mismatch"
                && d.message.contains("interoperability point")
        }),
        "a Table A.1 interop inequality must fire; report was: {report}"
    );
}

#[test]
fn lcr_msdo_aggregate_agreement_is_conforming() {
    // § 6.8.2 constraint 3 boundary: every aggregate field agrees. multistream_profile_idc
    // 0 (IOP 0), level 0, tier 0; config 0 allows profile 0, max_interop 0, level 0, tier 0.
    let agg = AggInfo {
        config_idc: 0,
        aggregate_level_idx: 0,
        max_tier_flag: 0,
        max_interop: 0,
    };
    let msdo = msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = lcr_msdo_stream(true, 1, 0b11, Some(agg), None, false, msdo);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/msdo-aggregate-mismatch"),
        "fully-agreeing aggregate info must not fire; report was: {report}"
    );
}

#[test]
fn lcr_msdo_substream_ptl_mismatch_is_flagged_both_orders() {
    // § 6.8.2 constraint 4: sub_stream_max_level[1] (4) != lcr_max_level_idx for
    // sub_xlayer_id 1 (7). Exact-equality semantics. Both arrival orders.
    let ptls = [
        GlobalPtl {
            seq_profile_idc: 0,
            max_level_idx: 0,
            tier_flag: 0,
            max_mlayer_count: 0,
        },
        GlobalPtl {
            seq_profile_idc: 0,
            max_level_idx: 7,
            tier_flag: 0,
            max_mlayer_count: 0,
        },
    ];
    for msdo_first in [true, false] {
        let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 4, 0)]);
        let data = lcr_msdo_stream(msdo_first, 1, 0b11, None, Some(&ptls), false, msdo);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report.errors().any(|d| {
                d.rule_id == "lcr/msdo-substream-ptl-mismatch"
                    && d.spec_section.as_deref() == Some("6.8.2")
            }),
            "a per-substream PTL mismatch must fire (msdo_first={msdo_first}); report: {report}"
        );
    }
}

#[test]
fn lcr_msdo_substream_ptl_agreement_is_conforming() {
    // § 6.8.2 constraint 4 boundary: exact equality on every dimension for each i.
    let ptls = [
        GlobalPtl {
            seq_profile_idc: 0,
            max_level_idx: 0,
            tier_flag: 0,
            max_mlayer_count: 0,
        },
        GlobalPtl {
            seq_profile_idc: 0,
            max_level_idx: 4,
            tier_flag: 0,
            max_mlayer_count: 0,
        },
    ];
    let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 4, 0)]);
    let data = lcr_msdo_stream(true, 1, 0b11, None, Some(&ptls), false, msdo);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/msdo-substream-ptl-mismatch"),
        "exact-matching per-substream PTL must not fire; report was: {report}"
    );
}

#[test]
fn lcr_msdo_doh_flag_mismatch_is_flagged() {
    // § 6.8.2 constraint 5: multistream_doh_constraint_flag (1) != lcr_doh_constraint_flag
    // (0). All headers monotonic so the DOH *requirement* does not also fire.
    let msdo = msdo_obu_configured(31, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = lcr_msdo_stream(true, 1, 0b11, None, None, false, msdo);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/msdo-doh-flag-mismatch" && d.spec_section.as_deref() == Some("6.8.2")
        }),
        "a DOH-flag mismatch must fire; report was: {report}"
    );
}

#[test]
fn lcr_msdo_doh_flag_agreement_is_conforming() {
    // § 6.8.2 constraint 5 boundary: both flags 1.
    let msdo = msdo_obu_configured(31, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = lcr_msdo_stream(true, 1, 0b11, None, None, true, msdo);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/msdo-doh-flag-mismatch"),
        "agreeing DOH flags must not fire; report was: {report}"
    );
}

#[test]
fn lcr_msdo_agreement_inert_for_unactivated_global_lcr() {
    // § 6.8.2: an observed-but-never-activated global LCR triggers no agreement
    // diagnostic. Here the headers use seq_lcr_id == 0 (no association), so the chain
    // never resolves the global LCR as activated even though a stream-count and DOH-flag
    // disagreement would otherwise fire.
    let global = global_lcr_obu_agreement(1, 0b111, None, None, false);
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(31, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(global);
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 0)); // seq_lcr_id == 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 0));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id.starts_with("lcr/msdo-")),
        "an unactivated global LCR triggers no § 6.8.2 agreement diagnostic; report: {report}"
    );
}

#[test]
fn lcr_doh_constraint_required_is_flagged() {
    // § 6.8.2 DOH requirement (lines 1619-1621): an activated header has
    // monotonic_output_order_flag == 0 while the activated global LCR's
    // lcr_doh_constraint_flag == 0. The MSDO's flag matches the global's (both 0) so the
    // §6.8.2 flag-mismatch does not also fire.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0)],
    ));
    data.extend(global);
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // monotonic 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, false, 1)); // monotonic 0 -> requires DOH
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/doh-constraint-required" && d.spec_section.as_deref() == Some("6.8.2")
        }),
        "the LCR DOH-constraint requirement must fire; report was: {report}"
    );
}

#[test]
fn lcr_doh_constraint_satisfied_by_flag_is_conforming() {
    // § 6.8.2 DOH requirement boundary: lcr_doh_constraint_flag == 1 satisfies it even
    // with a non-monotonic activated header. The MSDO flag is 1 too (so no flag mismatch).
    let global = global_lcr_obu_agreement(1, 0b11, None, None, true);
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(31, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(global);
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, false, 1)); // monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/doh-constraint-required"),
        "lcr_doh_constraint_flag == 1 satisfies the requirement; report was: {report}"
    );
}

#[test]
fn lcr_msdo_agreement_suppressed_under_external_hls() {
    // External HLS declaring a sequence header makes the activation chain unreliable, so
    // the § 6.8.2 agreement is suppressed even with a stream-count disagreement.
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let msdo = msdo_obu_configured(31, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]);
    let data = lcr_msdo_stream(true, 1, 0b111, None, None, false, msdo);
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report.errors().any(|d| d.rule_id.starts_with("lcr/msdo-")),
        "external HLS suppresses the § 6.8.2 agreement; report was: {report}"
    );
}

#[test]
fn lcr_msdo_agreement_inert_when_global_lcr_not_present_in_this_cmvs() {
    // Codex finding 1 (3393129738): a global LCR activated in an earlier CVS must not
    // leak into a *later* CMVS's § 6.8.2 evaluation when that CMVS contains no global-LCR
    // OBU. TU1/CVS1 opens a CMVS with a conforming MSDO (num_streams 2 == LcrMaxNumXLayer
    // Count 2) and global LCR id 1 activated by both layers' headers. TU2/CVS2 opens a NEW
    // CMVS (a changed MSDO: profile differs from TU1's) whose headers still reference
    // seq_lcr_id 1 (so the association chain resolves the *still-available* global LCR),
    // but no global-LCR OBU is re-sent. The TU2 MSDO declares num_streams 3, which would
    // disagree with the leaked record's LcrMaxNumXLayerCount 2 — yet § 6.8.2 must NOT fire,
    // because the global LCR is not present in TU2's CMVS. Pre-fix `activated_global_lcr`
    // resolves the leaked record via the live `global_lcr_records` map and the
    // stream-count mismatch fires falsely.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
    let mut data = temporal_delimiter_obu(); // TU1/CVS1: opens CMVS #1
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(global);
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // activates global LCR 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // activates global LCR 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
    data.extend(temporal_delimiter_obu()); // TU2/CVS2: a changed MSDO opens CMVS #2
    // num_streams_minus_2 + 2 == 3, profile 31 (differs from TU1's 0 → § 7.3.2 begin
    // condition 2 starts a new CMVS); three sub-streams. NO global LCR re-sent this CMVS.
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0), (2, 0, 0, 0)],
    ));
    // Headers redefined at the CVS boundary still reference seq_lcr_id 1 (the leaked
    // record is still available in-band), and are re-activated by the CLK frames.
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
    data.extend(temporal_delimiter_obu()); // close TU2
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id.starts_with("lcr/msdo-")),
        "a global LCR absent from this CMVS must not be evaluated against its MSDO; \
         report was: {report}"
    );
}

#[test]
fn lcr_msdo_agreement_uses_association_time_global_lcr_snapshot() {
    // Codex finding 2 (3393129741): the § 6.8.2 record is resolved from the
    // *association-time* snapshot, not a live lookup — a same-id global-LCR redefinition
    // after a header associated with the earlier revision must not retarget the agreement
    // at the later revision. Both revisions of global LCR id 1 have LcrMaxNumXLayerCount 2
    // (map 0b11) so the stream count matches; they differ only in lcr_doh_constraint_flag.
    // Revision A (doh 1) is observed before the headers, so the headers associate+activate
    // rev A. Revision B (doh 0) is re-sent after the headers (a redefinition). The MSDO's
    // multistream_doh_constraint_flag is 1, which AGREES with rev A but DISAGREES with rev
    // B. The agreement must compare against rev A (no mismatch). Pre-fix the live lookup
    // sees rev B and `lcr/msdo-doh-flag-mismatch` fires falsely.
    let global_a = global_lcr_obu_agreement(1, 0b11, None, None, true); // doh 1
    let global_b = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0 (redefine)
    let mut data = temporal_delimiter_obu();
    // MSDO multistream_doh_constraint_flag == 1 (agrees with rev A).
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(global_a); // rev A first
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // associates rev A
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // associates rev A
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
    data.extend(global_b); // rev B redefines id 1 AFTER the headers associated rev A
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/msdo-doh-flag-mismatch"),
        "the agreement must use the association-time rev A (doh 1), which agrees with the \
         MSDO; report was: {report}"
    );

    // Inverse: the MSDO agrees with rev B but disagrees with rev A. The diagnostic must
    // fire naming rev A's value (doh 1), because rev A is the associated record.
    let global_a = global_lcr_obu_agreement(1, 0b11, None, None, true); // doh 1 (associated)
    let global_b = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0
    let mut data = temporal_delimiter_obu();
    // MSDO multistream_doh_constraint_flag == 0 (agrees with rev B, disagrees with rev A).
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(global_a);
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1));
    data.extend(global_b);
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/msdo-doh-flag-mismatch"
                // rev A's lcr_doh_constraint_flag is 1; the message names the associated
                // record's value.
                && d.message.contains("lcr_doh_constraint_flag (1)")
        }),
        "the agreement must fire against the association-time rev A (doh 1); report was: \
         {report}"
    );
}

#[test]
fn lcr_doh_constraint_required_fires_without_msdo() {
    // Codex finding 3 (3393129743): the LCR DOH requirement is LCR-only — it must fire in
    // a global-LCR-only CMVS (no OBU_MSDO) when a confirmed activated header has
    // monotonic_output_order_flag == 0 and the activated global LCR's
    // lcr_doh_constraint_flag == 0. § 7.3.2 begin condition 3 (a CLK TU activating a global
    // LCR with no MSDO) opens such a CMVS. Pre-fix the resolver early-returns on the
    // missing MSDO, so the LCR-only requirement never fires.
    let global = global_lcr_obu_agreement(1, 0b1, None, None, false); // doh 0, single xlayer
    let mut data = temporal_delimiter_obu();
    data.extend(global); // global LCR present, no MSDO
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, false, 1)); // monotonic 0, activates LCR 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 (begin cond 3)
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/doh-constraint-required" && d.spec_section.as_deref() == Some("6.8.2")
        }),
        "the LCR DOH requirement is LCR-only and must fire without an MSDO; report was: \
         {report}"
    );
}

#[test]
fn lcr_doh_constraint_required_scoped_to_current_cmvs() {
    // Codex finding 4 (3393129745): the DOH loop must consider only sequence headers
    // activated within the CURRENT CMVS, not every frame-confirmed xlayer ever seen.
    // TU1/CVS1: xlayer 1 activates a header with monotonic_output_order_flag == 0 (frame-
    // confirmed), in a standalone CVS (no MSDO, no LCR → CMVS stays Outside) that ends
    // before the CMVS of interest. TU2/CVS2: opens a definitively-Inside CMVS on xlayer 0
    // (a CLK + MSDO begins it) whose own header is monotonic == 1, with an activated global
    // LCR whose lcr_doh_constraint_flag == 0. The non-monotonic xlayer-1 header belongs to
    // the earlier, ended CVS — it is NOT activated within TU2's CMVS, so no diagnostic may
    // fire. Pre-fix the loop iterates the whole-history frame_confirmed_xlayers set and
    // flags the leaked xlayer-1 header against TU2's global LCR. (The MSDO's
    // multistream_doh_constraint_flag is 1, so no § 6.6 check fires; the global LCR's count
    // and doh-flag match the MSDO so no § 6.8.2 agreement disagreement fires either.)
    let mut data = temporal_delimiter_obu(); // TU1/CVS1: a non-monotonic xlayer-1 header
    data.extend(seq_header_obu_lcr_ref(1, 5, 0, false, 0)); // xlayer 1, monotonic 0, no LCR
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 5)); // CLK xlayer 1 → confirms seq 5
    data.extend(temporal_delimiter_obu()); // TU2/CVS2: a fresh Inside CMVS on xlayer 0
    // A 2-xlayer global LCR (doh 0) and a matching 2-substream MSDO (doh 0): the count
    // matches (2 == 2) and the doh flags match, so neither the § 6.8.2 agreement nor the
    // § 6.6 MSDO DOH check fires — only the LCR DOH requirement is exercised, and it must
    // not fire because TU2's own header is monotonic == 1.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0, 2 xlayers
    data.extend(global);
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)])); // doh 0
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // xlayer 0, monotonic 1, LCR 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 (begin cond 1)
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "lcr/doh-constraint-required"),
        "a non-monotonic header from an earlier, ended CVS is outside this CMVS and must \
         not trigger the LCR DOH requirement; report was: {report}"
    );
}

#[test]
fn msdo_doh_constraint_required_scoped_to_current_cmvs() {
    // Codex finding 4 applied to the § 6.6 `msdo/doh-constraint-required` check
    // (resolve_deferred_doh_constraint): it also iterated the whole-history
    // frame_confirmed_xlayers set. TU1/CVS1: xlayer 1 activates a non-monotonic header in a
    // standalone CVS (no MSDO → CMVS stays Outside) that ends. TU2/CVS2: opens a
    // definitively-Inside CMVS on xlayer 0 (a CLK + MSDO) whose own header is monotonic ==
    // 1, with multistream_doh_constraint_flag == 0. The leaked xlayer-1 header is outside
    // TU2's CMVS, so no `msdo/doh-constraint-required` may fire.
    let mut data = temporal_delimiter_obu(); // TU1/CVS1: a non-monotonic xlayer-1 header
    data.extend(seq_header_obu_lcr_ref(1, 5, 0, false, 0)); // xlayer 1, monotonic 0
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 5)); // CLK xlayer 1 → confirms seq 5
    data.extend(temporal_delimiter_obu()); // TU2/CVS2: a fresh CMVS on xlayer 0
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)])); // doh 0
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 0)); // xlayer 0, monotonic 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "msdo/doh-constraint-required"),
        "a non-monotonic header from an earlier, ended CVS is outside this CMVS and must \
         not trigger the § 6.6 MSDO DOH requirement; report was: {report}"
    );
}

// -- § 7.3.2 cmvs/boundary-set-mismatch -------------------------------------

#[test]
fn cmvs_boundary_set_mismatch_is_flagged() {
    // § 7.3.2 boundary-set identity: a CMVS opens (TU1: MSDO + global LCR activated by
    // the header + CLK), then TU2 begins a new coded video sequence (a CLK) with NO
    // OBU_MSDO but WITH the activated global LCR. Under the MSDO-alone rules TU2 ends the
    // CMVS (end condition 2); under the MSDO+global-LCR rules it does not — the boundary
    // sets diverge, so cmvs/boundary-set-mismatch fires. The global LCR's
    // lcr_doh_constraint_flag matches the MSDO's (both 0), the xlayer_map count matches
    // num_streams (2), and aggregate/PTL info is absent, so no § 6.8.2 disagreement is
    // raised — only the boundary mismatch.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
    let mut data = temporal_delimiter_obu(); // temporal unit 1: opens the CMVS
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0)],
    ));
    data.extend(global.clone());
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
    data.extend(temporal_delimiter_obu()); // temporal unit 2 (no MSDO)
    // The global LCR is re-sent and re-activated by a same-id CLK; no MSDO this TU.
    data.extend(global);
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, ref seq 0
    data.extend(temporal_delimiter_obu()); // close TU2 via a boundary
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "cmvs/boundary-set-mismatch" && d.spec_section.as_deref() == Some("7.3.2")
        }),
        "the boundary-set divergence must fire; report was: {report}"
    );
}

#[test]
fn cmvs_boundary_set_no_mismatch_when_clk_carries_msdo() {
    // § 7.3.2: when the CLK-bearing TU2 also carries an OBU_MSDO, end condition 2 does
    // not apply under EITHER rule set (it begins a new CMVS instead), so the boundary
    // sets agree — no mismatch.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0)],
    ));
    data.extend(global.clone());
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(temporal_delimiter_obu()); // TU2: carries an MSDO
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0)],
    ));
    data.extend(global);
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "cmvs/boundary-set-mismatch"),
        "a CLK TU carrying an MSDO does not diverge the boundary sets; report was: {report}"
    );
}

#[test]
fn cmvs_boundary_set_silent_for_unactivated_global_lcr() {
    // § 7.3.2: when the global LCR in the CLK-bearing TU is only PRESENT but never
    // activated (the CMVS tracker routes that to Unknown), the divergence is undecidable
    // and must stay silent (lesson 12). Here TU2's CLK references seq_lcr_id 0, so no
    // global LCR is activated.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0)],
    ));
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 0)); // no LCR association
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0));
    data.extend(temporal_delimiter_obu()); // TU2: global LCR present but unactivated
    data.extend(global);
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK, still ref seq 0 (lcr 0)
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "cmvs/boundary-set-mismatch"),
        "an unactivated global LCR keeps the boundary check silent; report was: {report}"
    );
}

#[test]
fn cmvs_boundary_set_silent_when_activated_global_lcr_only_earlier_not_in_boundary_tu() {
    // Codex finding (3393274375): cmvs/boundary-set-mismatch over-fired. § 7.3.2 end
    // condition 2's divergence requires the BOUNDARY temporal unit itself to "have an
    // activated global layer configuration record" — a property of that temporal unit, not
    // of the whole CMVS window. Pre-fix the resolution found ANY activated global LCR
    // anywhere in the window, so a CMVS that activated a global LCR EARLIER over-fired at a
    // later CLK boundary TU that activated none of its own.
    //
    // TU1 opens the CMVS: MSDO (substreams 0,1), global LCR id 1 (map 0b11, doh 0) activated
    // by xlayer 0's header (seq_lcr_id 1, monotonic 1), CLK xlayer 0. xlayer 0's activated
    // global LCR remains chain-resolvable. TU2 is the boundary: it carries a global LCR OBU
    // (present → a boundary divergence CANDIDATE) and a CLK on xlayer 1 referencing a header
    // with seq_lcr_id 0 (NO LCR activation in TU2). xlayer 0 is NOT re-activated in TU2, so
    // the only global LCR activation lies in TU1, not the boundary TU. Both rule sets end
    // the CMVS at TU2 → no divergence → cmvs/boundary-set-mismatch must stay silent.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0, LcrXLayerID {0,1}
    let mut data = temporal_delimiter_obu(); // TU1: opens the CMVS, activates global LCR
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0)],
    ));
    data.extend(global.clone());
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // xlayer 0 activates global LCR 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
    data.extend(temporal_delimiter_obu()); // TU2: boundary — global LCR present but unactivated here
    data.extend(global); // global LCR present (divergence candidate), re-sent
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 0)); // xlayer 1 header, seq_lcr_id 0 (no LCR)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1, no LCR activation
    data.extend(temporal_delimiter_obu()); // close TU2 via a boundary
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "cmvs/boundary-set-mismatch"),
        "the boundary TU activates no global LCR of its own (only an earlier TU did), so \
         both boundary rule sets end the CMVS here and there is no mismatch; report was: \
         {report}"
    );
}

#[test]
fn lcr_only_cmvs_window_survives_to_later_frame_confirmed_activation() {
    // Codex finding (3393274378): an LCR-only CMVS opened via § 7.3.2 begin condition 3
    // (a CLK temporal unit that activates a global LCR with NO OBU_MSDO) is routed to
    // CmvsState::Unknown. A LATER temporal unit with no CLK fires no § 7.3.2 end condition
    // (end conditions 1/2 both require a CLK that "begins a new coded video sequence"), so
    // the CMVS window must be KEPT — pre-fix the window action returned Close, clearing the
    // window, and a later frame-confirmed non-monotonic activation in that LCR-only CMVS
    // was skipped by the deferred § 6.8.2 LCR-DOH check.
    //
    // TU1 opens the LCR-only CMVS: global LCR id 1 (lcr_doh_constraint_flag == 0) activated
    // by xlayer 0's header (seq_lcr_id 1, monotonic 1 → no DOH violation yet), CLK xlayer 0,
    // no MSDO. TU2 is a continuation (no CLK): xlayer 1's header (seq_lcr_id 1, monotonic 0)
    // is frame-confirmed by a regular tile group. With the window kept, xlayer 1's activation
    // lies in the CMVS, so lcr/doh-constraint-required must fire.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0, xlayers 0,1
    let mut data = temporal_delimiter_obu(); // TU1: opens the LCR-only CMVS (begin cond 3)
    data.extend(global);
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // xlayer 0, monotonic 1, activates LCR 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0, no MSDO
    data.extend(temporal_delimiter_obu()); // TU2: continuation (no CLK) — window must be kept
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, false, 1)); // xlayer 1, monotonic 0, refs LCR 1
    data.extend(frame_obu_direct_seq_ref_layer(7, 0, 0, 1, 1)); // regular tile group confirms xlayer 1
    data.extend(temporal_delimiter_obu()); // close TU2 via a boundary
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/doh-constraint-required" && d.spec_section.as_deref() == Some("6.8.2")
        }),
        "the LCR-only CMVS window must survive a non-CLK temporal unit so a later \
         non-monotonic activation triggers the § 6.8.2 LCR-DOH requirement; report was: \
         {report}"
    );
}

#[test]
fn lcr_msdo_agreement_flags_earlier_nonconforming_msdo_overwritten_by_later() {
    // Codex finding (3393274380): § 6.8.2 requires the MSDO↔global-LCR agreement to hold
    // for EVERY OBU_MSDO present in the CMVS, but the live `msdo_substream_max` is
    // last-wins. A non-conforming MSDO-A at the first RAP TU, then a conforming MSDO-B at a
    // later RAP TU of the SAME CMVS, must both be evaluated. Pre-fix the deferred resolution
    // read only the live (last-wins) MSDO record, so when MSDO-A's TU activates NO global
    // LCR (the agreement does not resolve there) and the global LCR is only activated LATER
    // in MSDO-B's TU, MSDO-B has already overwritten the live record — MSDO-A escapes.
    //
    // TU1 opens the CMVS (begin condition 1: CLK + MSDO-A) and activates NO global LCR:
    // xlayer 0's header references seq_lcr_id 0 (no LCR). So `activated_global_lcr()` is None
    // at TU1's boundary and MSDO-A is not evaluated yet. TU2 stays in the SAME CMVS (MSDO-B
    // shares every § 7.3.2 condition-2 key field with MSDO-A — only the RAP-permitted
    // sub_xlayer_id[i] differs — so it does not begin a new CMVS), introduces the global LCR
    // (map 0b11 → LcrXLayerID {0,1}), and activates it via xlayer 1 (seq_lcr_id 1, CLK). At
    // TU2's boundary the global LCR is activated and the live record is MSDO-B (conforming),
    // so pre-fix nothing fires. MSDO-A names sub_xlayer_id 2 (∉ {0,1}); accumulating every
    // in-window MSDO catches it. MSDO-B sits at TU2's CLK (a RAP), so § 7.3.8.2's non-RAP
    // identity rule does not fire on the sub_xlayer_id difference.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // LcrXLayerID {0,1}
    let mut data = temporal_delimiter_obu(); // TU1: opens the CMVS, NO global LCR activated
    // MSDO-A: sub_xlayer_ids [0, 2] — sub_xlayer_id 2 ∉ {0,1} → disagrees with the LCR.
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (2, 0, 0, 0)],
    ));
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 0)); // xlayer 0, seq_lcr_id 0 (no LCR)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
    data.extend(temporal_delimiter_obu()); // TU2: same CMVS, introduces+activates the global LCR
    // MSDO-B: same key fields, sub_xlayer_ids [0, 1] — all ∈ {0,1} → agrees. Only the
    // RAP-permitted sub_xlayer_id differs from MSDO-A, so no new CMVS begins.
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0)],
    ));
    data.extend(global); // global LCR observed before the header that references it
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // xlayer 1, seq_lcr_id 1 → global LCR 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1 -> activates LCR 1
    data.extend(temporal_delimiter_obu()); // close TU2 via a boundary
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "lcr/msdo-sub-xlayer-not-in-lcr"
                && d.spec_section.as_deref() == Some("6.8.2")
                // sub_xlayer_id 2 is carried ONLY by MSDO-A, so naming it proves the earlier
                // non-conforming MSDO-A was evaluated, not just the later conforming MSDO-B.
                && d.message.contains("sub_xlayer_id 2")
        }),
        "every MSDO in the CMVS must be evaluated, so the earlier non-conforming MSDO-A \
         (sub_xlayer_id 2 ∉ LcrXLayerID[]) must fire even though the later conforming MSDO-B \
         overwrote the live MSDO record; report was: {report}"
    );
}
