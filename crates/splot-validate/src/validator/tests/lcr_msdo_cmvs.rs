// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

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
        let mut d = global;
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
    let agg = AggInfo {
        config_idc: 0,
        aggregate_level_idx: 0,
        max_tier_flag: 0,
        max_interop: 0,
    };
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
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
    let mut data = temporal_delimiter_obu(); // TU1/CVS1: opens CMVS #1
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(global);
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // activates global LCR 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // activates global LCR 1
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 1)); // CLK xlayer 1
    data.extend(temporal_delimiter_obu()); // TU2/CVS2: a changed MSDO opens CMVS #2
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0), (2, 0, 0, 0)],
    ));
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
    let global_a = global_lcr_obu_agreement(1, 0b11, None, None, true); // doh 1
    let global_b = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0 (redefine)
    let mut data = temporal_delimiter_obu();
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

    let global_a = global_lcr_obu_agreement(1, 0b11, None, None, true); // doh 1 (associated)
    let global_b = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0
    let mut data = temporal_delimiter_obu();
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
                && d.message.contains("lcr_doh_constraint_flag (1)")
        }),
        "the agreement must fire against the association-time rev A (doh 1); report was: \
         {report}"
    );
}

#[test]
fn lcr_doh_constraint_required_fires_without_msdo() {
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
    let mut data = temporal_delimiter_obu(); // TU1/CVS1: a non-monotonic xlayer-1 header
    data.extend(seq_header_obu_lcr_ref(1, 5, 0, false, 0)); // xlayer 1, monotonic 0, no LCR
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 1, 5)); // CLK xlayer 1 → confirms seq 5
    data.extend(temporal_delimiter_obu()); // TU2/CVS2: a fresh Inside CMVS on xlayer 0
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

#[test]
fn cmvs_boundary_set_mismatch_is_flagged() {
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
    let mut data = temporal_delimiter_obu(); // temporal unit 1: opens the CMVS
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0)],
    ));
    data.extend_from_slice(&global);
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
    data.extend(temporal_delimiter_obu()); // temporal unit 2 (no MSDO)
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
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false);
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0)],
    ));
    data.extend_from_slice(&global);
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
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh 0, LcrXLayerID {0,1}
    let mut data = temporal_delimiter_obu(); // TU1: opens the CMVS, activates global LCR
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (1, 0, 0, 0)],
    ));
    data.extend_from_slice(&global);
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
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // LcrXLayerID {0,1}
    let mut data = temporal_delimiter_obu(); // TU1: opens the CMVS, NO global LCR activated
    data.extend(msdo_obu_configured(
        31,
        false,
        &[(0, 0, 0, 0), (2, 0, 0, 0)],
    ));
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 0)); // xlayer 0, seq_lcr_id 0 (no LCR)
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 0, 0, 0)); // CLK xlayer 0 -> opens CMVS
    data.extend(temporal_delimiter_obu()); // TU2: same CMVS, introduces+activates the global LCR
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
                && d.message.contains("sub_xlayer_id 2")
        }),
        "every MSDO in the CMVS must be evaluated, so the earlier non-conforming MSDO-A \
         (sub_xlayer_id 2 ∉ LcrXLayerID[]) must fire even though the later conforming MSDO-B \
         overwrote the live MSDO record; report was: {report}"
    );
}
