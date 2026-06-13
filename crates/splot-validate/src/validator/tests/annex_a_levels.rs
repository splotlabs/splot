// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// A level-2.0 sequence header with 10-bit frame dimensions (max 1024), plus a
/// frame of the given size that reaches `tile_info()` with the right single-tile
/// increment bits, all wrapped with a temporal delimiter prefix.
pub(in crate::validator::tests) fn level_2_0_stream(width: u32, height: u32) -> Vec<u8> {
    let seq = AnnexASeq {
        level_idx: 0,
        frame_dim_bits_minus_1: 9, // 10-bit frame dims (max 1024)
        max_frame_width_minus_1: 1023,
        max_frame_height_minus_1: 1023,
        ..AnnexASeq::base()
    };
    let mut data = td_and_annex_a_seq(seq);
    let (col, row) = annex_a_single_tile_increments(width, height);
    data.extend(annex_a_frame_obu(0, width, height, 10, col, row));
    data
}

#[test]
fn annex_a_frame_width_exceeds_max_h_size() {
    // Level 2.0 (LevelIdx 0) MaxHSize is 640. FrameWidth 641 (> 640) with a short
    // height stays under MaxPicSize 147456, isolating the MaxHSize limit (fail-past).
    let report = Validator::new(false).validate_bytes(&level_2_0_stream(641, 16));
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/frame-size-exceeds-level" && d.message.contains("MaxHSize")
        }),
        "report was: {report}"
    );
}

#[test]
fn annex_a_frame_at_max_h_size_passes() {
    // FrameWidth exactly 640 == MaxHSize passes (boundary, pass-at-limit).
    let report = Validator::new(false).validate_bytes(&level_2_0_stream(640, 16));
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/frame-size-exceeds-level"),
        "FrameWidth 640 == MaxHSize 640 must pass; report was: {report}"
    );
}

#[test]
fn annex_a_frame_pic_size_exceeds_level() {
    // Level 2.0 MaxPicSize is 147456. FrameWidth 640 x FrameHeight 640 = 409600 >
    // 147456 (both dimensions are within MaxHSize/MaxVSize 640, isolating the
    // pic-size limit).
    let report = Validator::new(false).validate_bytes(&level_2_0_stream(640, 640));
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/frame-size-exceeds-level" && d.message.contains("MaxPicSize")
        }),
        "report was: {report}"
    );
}

#[test]
fn annex_a_frame_below_minimum_dimension() {
    // FrameWidth < 16 violates the Annex A.4 minimum-dimension rule. An 8-wide frame
    // has sbCols == 1 -> no increment bit.
    let report = Validator::new(false).validate_bytes(&level_2_0_stream(8, 16));
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/frame-size-below-minimum"
                && d.spec_section.as_deref() == Some("A.4")
        }),
        "report was: {report}"
    );
}

#[test]
fn annex_a_frame_at_minimum_dimension_passes() {
    // FrameWidth == FrameHeight == 16 is exactly the minimum (boundary).
    let report = Validator::new(false).validate_bytes(&level_2_0_stream(16, 16));
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/frame-size-below-minimum"),
        "16x16 is the minimum and must pass; report was: {report}"
    );
}

#[test]
fn annex_a_level_31_disables_level_limits() {
    // seq_level_idx 31 (Maximum parameters): no level-based constraints, so a huge
    // frame that would blow past every level-2.0 limit must not be flagged.
    let seq = AnnexASeq {
        level_idx: 31,
        frame_dim_bits_minus_1: 11, // 12-bit dims (max 4096)
        max_frame_width_minus_1: 4095,
        max_frame_height_minus_1: 4095,
        ..AnnexASeq::base()
    };
    let mut data = td_and_annex_a_seq(seq);
    // Level 31 (NO_LEVEL) tile layout: max_tile_width_sb == sbCols, so a single tile
    // reads one column and one row stop bit for a 4000x4000 (63x63 superblock) frame.
    data.extend(annex_a_frame_obu(0, 4000, 4000, 12, 1, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("annex-a/frame-size-")),
        "level 31 disables all level-limit checks; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/tile-count-exceeds-level"),
        "level 31 disables the tile-count check too; report was: {report}"
    );
}

#[test]
fn annex_a_reserved_level_disables_level_limits() {
    // A reserved seq_level_idx (22-30) is not in Tables A.8/A.9, so the level-limit
    // checks are disabled (the reserved-level value-space error still fires).
    let seq = AnnexASeq {
        level_idx: 22,
        frame_dim_bits_minus_1: 9,
        max_frame_width_minus_1: 1023,
        max_frame_height_minus_1: 1023,
        ..AnnexASeq::base()
    };
    let mut data = td_and_annex_a_seq(seq);
    // A reserved seq_level_idx has no defined tile scaling, so the frame's tile_info()
    // parse stops as Unimplemented and the frame-core checks are skipped — the
    // level-limit checks never run regardless of the (here unreached) tile bits.
    data.extend(annex_a_frame_obu(0, 640, 640, 10, 1, 1)); // would exceed level 2.0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/frame-size-exceeds-level"),
        "a reserved level has no level limits; report was: {report}"
    );
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "annex-a/level-reserved"),
        "the reserved-level value-space error still fires; report was: {report}"
    );
}

// --- ops_level_idx reserved (Annex A.4 Table A.7) ---

#[test]
fn annex_a_flags_reserved_ops_level_idx() {
    // A global OPS carrying ops_level_idx 25 (reserved 22-30) for one extended layer.
    let mut data = temporal_delimiter_obu();
    data.extend(ops_obu_with_level(25));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/level-reserved"
                && d.message.contains("ops_level_idx")
                && d.spec_section.as_deref() == Some("A.4")
        }),
        "report was: {report}"
    );
}

#[test]
fn annex_a_accepts_valid_ops_level_idx() {
    // ops_level_idx 4 (level 4.0) is a defined level — no level-reserved error.
    let mut data = temporal_delimiter_obu();
    data.extend(ops_obu_with_level(4));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/level-reserved"),
        "ops_level_idx 4 is a defined level; report was: {report}"
    );
}

#[test]
fn annex_a_flags_high_tier_below_4_0_in_ops() {
    // The reachable high-tier-below-4.0 arm (mirror lines 443-451 + the Table A.9
    // NOTE): the OPS PTL signals ops_tier_flag unconditionally (§ 5.11.2), so a High
    // tier (ops_tier_flag == 1) with ops_level_idx 3 (level 3.1, below 4.0) is a real
    // case the seq-header arm cannot reach. Exactly one advisory warning fires.
    let mut data = temporal_delimiter_obu();
    data.extend(ops_obu_with_level_tier(3, true));
    let report = Validator::new(false).validate_bytes(&data);
    let high_tier: Vec<_> = report
        .warnings()
        .filter(|d| d.rule_id == "annex-a/high-tier-below-4-0")
        .collect();
    assert_eq!(
        high_tier.len(),
        1,
        "exactly one high-tier-below-4.0 warning; report was: {report}"
    );
    let warning = high_tier[0];
    assert_eq!(warning.spec_section.as_deref(), Some("A.4"));
    assert!(
        warning.message.contains("ops_tier_flag") && warning.message.contains("ops_level_idx 3"),
        "message names ops_tier_flag/ops_level_idx; report was: {report}"
    );
}

#[test]
fn annex_a_accepts_high_tier_at_4_0_in_ops() {
    // ops_tier_flag == 1 at ops_level_idx 4 (level 4.0) is allowed (Table A.9 NOTE:
    // 4.0 and above) — no high-tier-below-4.0 diagnostic.
    let mut data = temporal_delimiter_obu();
    data.extend(ops_obu_with_level_tier(4, true));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .warnings()
            .all(|d| d.rule_id != "annex-a/high-tier-below-4-0"),
        "High tier at level 4.0 is allowed; report was: {report}"
    );
}

#[test]
fn annex_a_accepts_main_tier_below_4_0_in_ops() {
    // ops_tier_flag == 0 (Main) at ops_level_idx 3 (below 4.0) is fine — the NOTE
    // only restricts the High tier — so no high-tier-below-4.0 diagnostic.
    let mut data = temporal_delimiter_obu();
    data.extend(ops_obu_with_level_tier(3, false));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .warnings()
            .all(|d| d.rule_id != "annex-a/high-tier-below-4-0"),
        "Main tier below 4.0 is fine; report was: {report}"
    );
}

/// A local OPS OBU (xlayer 0, `ops_cnt == 1`, `ops_ptl_present`) whose single
/// operating point's `ops_seq_profile_tier_level_info()` (§ 5.11.2) signals
/// `ops_level_idx == level_idx` with `ops_tier_flag == 0` (Main).
pub(in crate::validator::tests) fn ops_obu_with_level(level_idx: u32) -> Vec<u8> {
    ops_obu_with_level_tier(level_idx, false)
}

/// A local OPS OBU (xlayer 0, `ops_cnt == 1`, `ops_ptl_present`) whose single
/// operating point's `ops_seq_profile_tier_level_info()` (§ 5.11.2) signals
/// `ops_level_idx == level_idx` and `ops_tier_flag == high_tier`, with
/// `ops_seq_profile_idc == 0`.
pub(in crate::validator::tests) fn ops_obu_with_level_tier(
    level_idx: u32,
    high_tier: bool,
) -> Vec<u8> {
    ops_obu_with_profile_level_tier(0, level_idx, high_tier)
}

/// A local OPS OBU (xlayer 0, `ops_cnt == 1`, `ops_ptl_present`) whose single
/// operating point's `ops_seq_profile_tier_level_info()` (§ 5.11.2) signals
/// `ops_seq_profile_idc == profile_idc`, `ops_level_idx == level_idx`, and
/// `ops_tier_flag == high_tier`. Modeled on `local_ops_obu` (OBU type 18); the
/// per-op `ops_data_size` is the byte-aligned body length. Unlike the sequence
/// header, the OPS PTL carries `ops_tier_flag` unconditionally, so High tier can be
/// signaled at any level here.
pub(in crate::validator::tests) fn ops_obu_with_profile_level_tier(
    profile_idc: u32,
    level_idx: u32,
    high_tier: bool,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(0); // ops_reset_flag
    bits.f(0, 4); // ops_id
    bits.f(1, 3); // ops_cnt == 1
    bits.f(0, 4); // ops_priority
    bits.f(0, 7); // ops_intent
    bits.bit(0); // ops_intent_present_flag
    bits.bit(1); // ops_ptl_present_flag
    bits.bit(0); // ops_color_info_present_flag
    bits.f(0, 2); // ops_reserved_2bits (local OPS)
    // operating_point_payload(0):
    let mut body = Bits::default();
    // ops_seq_profile_tier_level_info() (§ 5.11.2).
    body.f(profile_idc, 5); // ops_seq_profile_idc
    body.f(level_idx, 5); // ops_level_idx
    body.bit(u8::from(high_tier)); // ops_tier_flag
    body.f(0, 3); // ops_mlayer_count
    body.f(0, 2); // ops_ptl_reserved_2bits
    body.bit(0); // ops_decoder_model_info_for_this_op_present_flag
    body.bit(0); // ops_initial_display_delay_present_flag
    body.f(0, 8); // ops_mlayer_info(): ops_mlayer_map = 0
    body.align();
    let body_bytes = (body.bits.len() / 8) as u32;
    bits.f(body_bytes, 8); // ops_data_size (leb128, single byte for len < 128)
    bits.bits.extend_from_slice(&body.bits);
    annex_b_obu_with_header(&layer_obu_header(18, 0, 0, 0), &finish_extensible(bits))
}

#[test]
fn annex_a_flags_reserved_ops_seq_profile_idc() {
    // A local OPS carrying ops_seq_profile_idc 7 (reserved 5-30) for one extended
    // layer must be flagged as a reserved profile (§ 6.10.4 maps the OPS-derived
    // profile id onto Annex A.2 Table A.1).
    let mut data = temporal_delimiter_obu();
    data.extend(ops_obu_with_profile_level_tier(7, 4, false));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "annex-a/profile-reserved"
                && d.message.contains("ops_seq_profile_idc")
                && d.spec_section.as_deref() == Some("A.2")
        }),
        "report was: {report}"
    );
}

#[test]
fn annex_a_accepts_valid_ops_seq_profile_idc() {
    // ops_seq_profile_idc 0 is a defined profile — no profile-reserved error.
    let mut data = temporal_delimiter_obu();
    data.extend(ops_obu_with_profile_level_tier(0, 4, false));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "annex-a/profile-reserved"),
        "ops_seq_profile_idc 0 is a defined profile; report was: {report}"
    );
}
