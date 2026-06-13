// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn validator_frame_header_copy_record_cleared_by_sef_opening_new_frame() {
    // Regression (codex round-9 F2): a completed first tile group records its
    // NumFrameHeaderBits for the (xlayer, mlayer, tlayer) triple. A SEF in the SAME triple
    // is its own single-OBU coded frame (§ 7.3.3) — it OPENS a new coded frame
    // (OpensNewUnit), ending the tile coded frame whose header is recorded. The segmenter
    // then routes a following flag-0 (is_first_tile_group == 0) tile group as CONTINUING
    // the SEF coded frame (the frame-unit/sef-single-obu case), so it carries no
    // frame_header_copy() of its own. Pre-fix the SEF (not a tile group) returned early
    // before clearing the record, leaving the STALE tile-frame record alive; the flag-0
    // tile group then paired against it and false-positived frame-header/copy-bits-mismatch.
    // Post-fix the SEF's OpensNewUnit boundary clears the record, so the flag-0 tile group
    // finds none and stays silent on copy-bits-*.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(clk_first_tile_group()); // records NumFrameHeaderBits for the triple
    data.extend(conformant_sef_same_triple()); // SEF: own coded frame, same triple
    // A readable flag-0 non-first tile group whose copy region does NOT match the recorded
    // first header. The segmenter treats it as continuing the SEF coded frame.
    let mut mismatched = complete_intra_clk_frame_header_body().drain_bits();
    mismatched[2] ^= 1;
    data.extend(clk_non_first_tile_group(&mismatched));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id.starts_with("frame-header/copy-bits-")),
        "a SEF opening a new coded frame must clear the prior tile frame's record so a \
         later flag-0 tile group does not pair against it; report was: {report}"
    );
    // Control: the segmenter's own sef-single-obu diagnostic still fires for the
    // flag-0-after-SEF continuation (its existing behavior is unchanged by the copy fix).
    assert!(
        has_frame_unit_error(&report, "frame-unit/sef-single-obu"),
        "the flag-0 tile group continuing the SEF coded frame must still fire \
         sef-single-obu; report was: {report}"
    );
}

/// A multi-frame header OBU (type 3) at xlayer 0 with `mfh_seq_header_id` 0 and
/// `mfhId` 1 (`mfh_id_minus_1` 0), carrying an `mfh_frame_size_present_flag`
/// payload whose `mfh_frame_width_minus_1` / `mfh_frame_height_minus_1` derive the
/// given `FrameWidth` / `FrameHeight` (AV2 § 5.7). No segmentation info, no
/// deblocking update. A `cur_mfh_id == 1` frame with `frame_size_override_flag == 0`
/// then derives its FrameWidth/FrameHeight from this record (mirror :5767).
pub(in crate::validator::tests) fn mfh_obu_with_frame_size(width: u32, height: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(0); // mfh_seq_header_id 0
    bits.uvlc(0); // mfh_id_minus_1 -> mfhId = 1
    bits.bit(1); // mfh_frame_size_present_flag
    bits.f(8 - 1, 4); // mfh_frame_width_bits_minus_1 -> 8-bit width field
    bits.f(8 - 1, 4); // mfh_frame_height_bits_minus_1 -> 8-bit height field
    bits.f(width - 1, 8); // mfh_frame_width_minus_1 -> FrameWidth = width
    bits.f(height - 1, 8); // mfh_frame_height_minus_1 -> FrameHeight = height
    bits.bit(0); // mfh_deblocking_filter_update
    bits.bit(0); // mfh_seg_info_present_flag -> fully parsed
    bits.bit(0); // obu_extension_flag = 0
    bits.bit(1); // trailing_one_bit
    annex_b_obu(0x0C, &bits.into_bytes())
}

/// A multi-frame header OBU (type 3) at xlayer 0 with `mfh_seq_header_id` 0 and
/// `mfhId` 1, with `mfh_frame_size_present_flag == 0` (no stored dimensions). AV2
/// § 5.18.2 (:4101) infers the default dims to the sequence maxima at consumption,
/// so the §6.17.2 `mfh_frame_width_minus_1 <= max_frame_width_minus_1` bound has no
/// stored value to compare and is trivially satisfied.
pub(in crate::validator::tests) fn mfh_obu_without_frame_size() -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(0); // mfh_seq_header_id 0
    bits.uvlc(0); // mfh_id_minus_1 -> mfhId = 1
    bits.bit(0); // mfh_frame_size_present_flag == 0 (no stored dims)
    bits.bit(0); // mfh_deblocking_filter_update
    bits.bit(0); // mfh_seg_info_present_flag -> fully parsed
    bits.bit(0); // obu_extension_flag = 0
    bits.bit(1); // trailing_one_bit
    annex_b_obu(0x0C, &bits.into_bytes())
}

/// A `cur_mfh_id == 1`, `frame_size_override_flag == 0` CLK frame (AV2 § 5.18.2)
/// whose FrameWidth/FrameHeight come from the resolved MFH record's default
/// dimensions (no explicit width/height bits). `col_increment_bits` is the number
/// of `tile_info()` column-increment bits the derived FrameWidth requires.
pub(in crate::validator::tests) fn mfh_backed_clk_default_size(col_increment_bits: u32) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(1); // cur_mfh_id == 1 (resolves through the MFH; no seq id field follows)
    fb.bit(0); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag == 0 -> MFH default dims
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    // frame_size(): no bits on the non-override path (dims come from the MFH).
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, col_increment_bits);
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// A `cur_mfh_id == 1`, `frame_size_override_flag == 1` CLK frame (AV2 § 5.18.2)
/// whose FrameWidth/FrameHeight come from explicit `frame_width_minus_1` /
/// `frame_height_minus_1` bits (`width` / `height`), ignoring the resolved MFH's
/// stored default dimensions. `col_increment_bits` is the number of `tile_info()`
/// column-increment bits the overridden FrameWidth requires. The explicit width
/// field is the sequence `frame_width_bits` wide (8 for [`FrameCoreSeq::base`]).
pub(in crate::validator::tests) fn mfh_backed_clk_override_size(
    width: u32,
    height: u32,
    col_increment_bits: u32,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(1); // cur_mfh_id == 1 (resolves through the MFH; no seq id field follows)
    fb.bit(0); // immediate_output_frame
    fb.bit(1); // frame_size_override_flag == 1 -> explicit dims override the MFH
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    fb.f(width - 1, 8); // frame_width_minus_1 (f(frame_width_bits == 8))
    fb.f(height - 1, 8); // frame_height_minus_1 (f(frame_height_bits == 8))
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, col_increment_bits);
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

#[test]
fn validator_flags_mfh_stored_frame_size_exceeds_sequence_max_on_override() {
    // Regression for the §6.17.2 stored-MFH bound: a cur_mfh_id > 0 frame overrides
    // to in-range dims (16x16 == max), so the derived FrameWidth is conformant and
    // frame-header/frame-size-exceeds-sequence-max (§6.17.4.1) must stay silent. But
    // the referenced MFH's STORED mfh_frame_width_minus_1 derives FrameWidth 256 >
    // max_frame_width 16, which §6.17.2 (mirror :4348) bounds independently of
    // frame_size_override_flag, after load_sequence_header. Pre-fix this was silent
    // (the round-1 check fires only when the MFH dims flow into core.frame_size).
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(256, 8)); // stored dims 256x8 (> max 16x16)
    // Override to 16x16 (in range); 16-wide frame: sbCols == 1, no column increment.
    data.extend(mfh_backed_clk_override_size(16, 16, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
    // The override dims are in range, so the §6.17.4.1 derived check stays silent.
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "the in-range override must not fire the derived frame-size check; report was: {report}"
    );
}

#[test]
fn validator_accepts_mfh_stored_frame_size_within_sequence_max_on_override() {
    // Negative control: a conformant MFH (stored dims 16x16 == max) backing an
    // in-range override frame must stay silent on both rules.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(16, 16));
    data.extend(mfh_backed_clk_override_size(16, 16, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
}

#[test]
fn validator_does_not_flag_mfh_omitted_frame_size_on_override() {
    // Omitted-size control: an MFH with no mfh_frame_size payload infers its default
    // dims to the sequence maxima (trivially in range), so the §6.17.2 stored-dims
    // check has nothing to compare and stays silent even when the frame overrides.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_without_frame_size());
    data.extend(mfh_backed_clk_override_size(16, 16, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "an omitted MFH size infers the maxima and must stay silent; report was: {report}"
    );
}

#[test]
fn validator_does_not_double_fire_mfh_frame_size_on_default_path() {
    // No-double-fire control for the override==0 path: the MFH stored dims flow into
    // core.frame_size verbatim, so the §6.17.2 stored-MFH check is the single home —
    // frame-header/mfh-frame-size-exceeds-sequence-max fires exactly once and the
    // §6.17.4.1 derived frame-header/frame-size-exceeds-sequence-max does NOT also
    // fire on the same dims.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(256, 8)); // stored dims 256x8 (> max 16x16)
    // 256-wide frame: sbCols == 4, so tile_info() reads one column increment bit.
    data.extend(mfh_backed_clk_default_size(1));
    let report = Validator::new(false).validate_bytes(&data);
    assert_eq!(
        report
            .errors()
            .filter(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max")
            .count(),
        1,
        "the stored-MFH check is the single home for the default path; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "the derived check must defer to the stored-MFH check on the default path; \
         report was: {report}"
    );
}

#[test]
fn validator_does_not_flag_mfh_stored_frame_size_when_mfh_unresolvable() {
    // Unresolvable-MFH control: with no in-band MFH backing cur_mfh_id == 1, the
    // §6.17.2 stored-dims check has no record to read and stays silent (the existing
    // guard), exactly like the derived check.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    // No MFH OBU recorded; the override CLK still references cur_mfh_id == 1.
    data.extend(mfh_backed_clk_override_size(16, 16, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "an unresolvable MFH must keep the silent behavior; report was: {report}"
    );
}

#[test]
fn validator_flags_mfh_default_frame_size_exceeds_sequence_max() {
    // Regression for the cur_mfh_id > 0 default-size path: with
    // frame_size_override_flag == 0, FrameWidth/Height come from the MFH record
    // (mirror :5767). The MFH carries FrameWidth 256 > max_frame_width 16, so the
    // resolved frame must fire the §6.17.2 stored-MFH bound
    // (mfh_frame_width_minus_1 <= max_frame_width_minus_1, mirror :4348), homed in
    // frame-header/mfh-frame-size-exceeds-sequence-max. The default-path dims flow
    // verbatim into core.frame_size, so this is the single home (the §6.17.4.1
    // derived check defers — see validator_does_not_double_fire_mfh_frame_size_on_default_path).
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(256, 8));
    // 256-wide frame: sbCols == 4, so tile_info() reads one column increment bit.
    data.extend(mfh_backed_clk_default_size(1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_mfh_default_frame_size_within_sequence_max() {
    // Negative control: a resolvable, conformant MFH (FrameWidth 16 == max,
    // FrameHeight 16 == max) backing the same default-size frame must stay silent.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(16, 16));
    data.extend(mfh_backed_clk_default_size(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "report was: {report}"
    );
}

#[test]
fn validator_does_not_flag_mfh_default_frame_size_when_mfh_unresolvable() {
    // Unresolvable-MFH control: no in-band MFH backs cur_mfh_id == 1, so the core
    // parse stops before frame_size() (no guessing) and the frame-size check cannot
    // fire — the pre-fix early-stop behavior is preserved for this case.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    // No MFH OBU recorded; the CLK still references cur_mfh_id == 1.
    data.extend(mfh_backed_clk_default_size(1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "an unresolvable MFH must keep the early-stop behavior; report was: {report}"
    );
}

#[test]
fn validator_flags_mfh_stored_frame_size_exceeds_sequence_max_on_truncated_frame() {
    // F1 regression: the §6.17.2 stored-dims bound (mfh_frame_width_minus_1 <=
    // max_frame_width_minus_1, mirror :4348) depends ONLY on the resolved MFH
    // record and the active sequence maxima — it is decidable at the
    // load_sequence_header point regardless of how far the referencing frame
    // header parses. A `cur_mfh_id == 1` frame whose payload is truncated right
    // after the prefix (`frame_obu_mfh_ref` codes only is_first_tile_group +
    // cur_mfh_id, so parse_frame_core returns None) must still fire the stored-MFH
    // check: the referenced MFH stores FrameWidth 256 > max_frame_width 16.
    // Pre-fix the check ran only past the `let Some(core) = parse_frame_core(..)`
    // early-stop, so this truncated frame was silently skipped.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(256, 8)); // stored dims 256x8 (> max 16x16)
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1)); // truncated after the prefix
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "a truncated frame referencing an oversized-stored-dims MFH must still fire \
         the §6.17.2 stored-dims check; report was: {report}"
    );
}

#[test]
fn validator_accepts_mfh_within_sequence_max_on_truncated_frame() {
    // F1 negative control: a conformant MFH (stored dims 16x16 == max) backing a
    // truncated `cur_mfh_id == 1` frame must stay silent on the §6.17.2
    // stored-dims rule for this id — the hoist must not introduce a false positive
    // when the core parse fails but the stored dims are in range.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(16, 16)); // stored dims 16x16 == max
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1)); // truncated after the prefix
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "a conformant-MFH truncated frame must stay silent; report was: {report}"
    );
}

#[test]
fn validator_flags_both_frame_size_rules_when_override_repeats_mfh_out_of_range_dims() {
    // F2 regression: an override==1 frame that explicitly codes the SAME
    // out-of-range dims the MFH stores is a genuine §6.17.4.1 violation of its own
    // explicit frame_width/height_minus_1 fields, distinct from the §6.17.2
    // stored-MFH violation. The no-double-fire suppression must key on the parsed
    // PATH (frame_size_override_flag), not on dimension equality: with override==1
    // BOTH frame-header/frame-size-exceeds-sequence-max (§6.17.4.1, the explicit
    // fields) and frame-header/mfh-frame-size-exceeds-sequence-max (§6.17.2, the
    // stored dims) must fire. Pre-fix the value-equality `derived_is_mfh_default`
    // wrongly suppressed the §6.17.4.1 check because the numbers matched.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    data.extend(mfh_obu_with_frame_size(256, 8)); // stored dims 256x8 (> max 16x16)
    // Override to the SAME out-of-range dims 256x8 (256-wide -> one column increment).
    data.extend(mfh_backed_clk_override_size(256, 8, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/mfh-frame-size-exceeds-sequence-max"),
        "the §6.17.2 stored-MFH check must fire; report was: {report}"
    );
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-size-exceeds-sequence-max"),
        "an override==1 frame's explicit out-of-range dims are a separate §6.17.4.1 \
         violation that must fire even when they equal the stored MFH dims; \
         report was: {report}"
    );
}

// OBU header bytes: obu_type << 2. 0x14 = OBU_OPEN_LOOP_KEY (5), 0x1c =
// OBU_REGULAR_TILE_GROUP (7).
pub(in crate::validator::tests) const OLK_HEADER: u8 = 0x14;
pub(in crate::validator::tests) const RTG_HEADER: u8 = 0x1c;

#[test]
fn validator_flags_frame_to_refresh_out_of_range() {
    // Compact refresh with NumRefFrames == 6: frame_to_refresh == 6 (>= 6) yields
    // refresh_frame_flags == 1 << 6, a slot at/beyond NumRefFrames (AV2 § 6.17.2).
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5, // NumRefFrames == 6
        enable_short_refresh_frame_flags: true,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
    fb.bit(0); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag (default dims)
    fb.f(0, 1); // order_hint
    fb.bit(1); // has_refresh_frame_flags
    fb.f(6, 3); // frame_to_refresh == 6 (CeilLog2(6) == 3) -> refresh = 1 << 6
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-to-refresh-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_reserved_ref_long_term_id() {
    // RAS with long_term_frame_id_bits == 4 and a ref_long_term_id of 15 == the
    // reserved (1 << 4) - 1 (AV2 § 6.17.2).
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // restricted_prediction_switch
    fb.f(1, 3); // num_key_ref_frames == 1
    fb.f(15, 4); // ref_long_term_id[0] == (1 << 4) - 1 (reserved)
    data.extend(annex_b_obu(RAS_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/ref-long-term-id-reserved"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_zero_refresh_on_deferred_output() {
    // OLK forces immediate_output_frame == 0; refresh_frame_flags == 0 then violates
    // AV2 § 6.17.2 (a deferred-output frame must update a reference slot).
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    // OLK: long_term_id_plus_1 f(0) (no bits); immediate forced 0; implicit -> 0
    fb.bit(0); // frame_size_override_flag (default dims)
    fb.f(0, 1); // order_hint
    fb.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8) == 0
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(OLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/refresh-frame-flags-zero-on-deferred-output"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_still_picture_non_key_frame() {
    // still_picture == 1 requires a KEY_FRAME; an INTRA_ONLY frame violates
    // AV2 § 6.17.2.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        still_picture: true,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY (not KEY_FRAME)
    fb.bit(1); // immediate_output_frame == 1 (isolate the frame-type violation)
    fb.bit(0); // frame_size_override_flag (default dims)
    fb.f(0, 1); // order_hint
    fb.f(1, 8); // refresh_frame_flags f(8) == 1 (nonzero, not all-slots)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/still-picture-requires-key-frame"),
        "report was: {report}"
    );
}

#[test]
fn validator_flags_intra_only_refresh_all_slots() {
    // INTRA_ONLY with NumRefFrames == 2 must not refresh every slot
    // (refresh_frame_flags != (1 << 2) - 1) (AV2 § 6.17.2).
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 1, // NumRefFrames == 2
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
    fb.bit(1); // immediate_output_frame == 1
    fb.bit(0); // frame_size_override_flag (default dims)
    fb.f(0, 1); // order_hint
    fb.f(0b11, 2); // refresh_frame_flags f(NumRefFrames == 2) == all slots
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/intra-only-refresh-all-slots"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_in_range_frame_to_refresh() {
    // The same compact-refresh frame with frame_to_refresh == 5 (< NumRefFrames 6)
    // must not be flagged.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 5,
        enable_short_refresh_frame_flags: true,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
    fb.bit(0); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint
    fb.bit(1); // has_refresh_frame_flags
    fb.f(5, 3); // frame_to_refresh == 5 (< NumRefFrames 6)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/frame-to-refresh-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_non_reserved_ref_long_term_id() {
    // ref_long_term_id == 14 != the reserved (1 << 4) - 1 == 15 must not be flagged.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        long_term_frame_id_bits: 4,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // restricted_prediction_switch
    fb.f(1, 3); // num_key_ref_frames == 1
    fb.f(14, 4); // ref_long_term_id[0] == 14 (not reserved)
    data.extend(annex_b_obu(RAS_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/ref-long-term-id-reserved"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_nonzero_refresh_on_deferred_output() {
    // An OLK frame (immediate_output_frame == 0) with refresh_frame_flags != 0 is
    // conformant.
    let mut data = td_and_frame_core_seq(FrameCoreSeq::base());
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint
    fb.f(1, 8); // refresh_frame_flags f(8) == 1 (nonzero)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(OLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/refresh-frame-flags-zero-on-deferred-output"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_still_picture_key_frame() {
    // A still_picture sequence with a KEY_FRAME (CLK) and immediate_output_frame == 1
    // is conformant.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        still_picture: true,
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // immediate_output_frame == 1
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(CLK_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/still-picture-requires-key-frame"),
        "report was: {report}"
    );
}

#[test]
fn validator_accepts_intra_only_partial_refresh() {
    // An INTRA_ONLY frame whose refresh_frame_flags is not all slots is conformant.
    let mut data = td_and_frame_core_seq(FrameCoreSeq {
        num_ref_frames_minus_1: 1, // NumRefFrames == 2
        ..FrameCoreSeq::base()
    });
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // frame_is_inter == 0 -> INTRA_ONLY
    fb.bit(1); // immediate_output_frame == 1
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint
    fb.f(0b01, 2); // refresh_frame_flags == 1 (not all slots)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    data.extend(annex_b_obu(RTG_HEADER, &fb.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/intra-only-refresh-all-slots"),
        "report was: {report}"
    );
}

// --- Frame tile-info / QM-reference diagnostics (AV2 § 6.17.7.2 / § 6.17.6.2) ---

/// Appends a CLK frame-header bit fixture from the activation prefix through
/// `disable_cdf_update`, with `frame_size_override_flag == 1` and the given
/// dimensions, leaving `fb` positioned at `tile_info()` (AV2 § 5.18.2). The
/// dimension fields are `f(width_bits)` / `f(height_bits)`
/// (`frame_*_bits_minus_1 + 1` from the sequence).
pub(in crate::validator::tests) fn clk_frame_until_tile_info(
    fb: &mut Bits,
    width: u32,
    height: u32,
    bits: (u32, u32),
) {
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // immediate_output_frame
    fb.bit(1); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    fb.f(width - 1, bits.0); // frame_width_minus_1
    fb.f(height - 1, bits.1); // frame_height_minus_1
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
}

/// Appends the post-`tile_info()` § 5.18.2 structures with every optional read
/// disabled: `base_q_idx` f(9) (§ 5.18.6.1), `segmentation_enabled = 0`
/// (§ 5.18.7.1), `using_qmatrix = 0` (§ 5.18.6.2), `delta_q_present = 0`
/// (§ 5.18.7.8); the lossless tail then reads no bits.
pub(in crate::validator::tests) fn quant_seg_tail(fb: &mut Bits) {
    fb.f(100, 9); // base_q_idx f(9) (10-bit sequence)
    fb.bit(0); // segmentation_enabled
    fb.bit(0); // using_qmatrix
    fb.bit(0); // delta_q_present
}

/// Encodes `ns(n)` value `0` (AV2 § 4.11.6): `w = FloorLog2(n) + 1`,
/// `m = (1 << w) - n`; `0 < m` always holds, so the encoding is `f(0, w - 1)`.
pub(in crate::validator::tests) fn ns_zero(fb: &mut Bits, n: u32) {
    let w = 32 - n.leading_zeros();
    fb.f(0, w - 1);
}
