// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

// === AV2 § 7.23 reference-frame buffer state model =====================

/// A show-existing-frame OBU referencing `slot` (`frame_to_show_map_idx`), against a
/// base sequence (NumRefFrames == 8 -> CeilLog2 == 3 bits, OrderHintBits == 1, no
/// film grain). `derive_sef_order_hint == 1` so no `sef_order_hint` is read, then the
/// §5.2.3 trailing one bit. The SEF arm sets `refresh_frame_flags = 0`, so it updates
/// no slot but DISPLAYS `slot` — making `RefValid[slot]` checkable (§6.17.2).
pub(in crate::validator::tests) const REF_SEF_HEADER: u8 = REGULAR_SEF_HEADER;

pub(in crate::validator::tests) fn ref_sef(slot: u32) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.f(slot, 3); // frame_to_show_map_idx f(CeilLog2(8) == 3)
    fb.bit(1); // derive_sef_order_hint == 1 -> no sef_order_hint
    // film_grain_params_present == false -> film_grain_config() reads no bits.
    fb.bit(1); // §5.2.3 trailing_one_bit
    annex_b_obu(REF_SEF_HEADER, &fb.into_bytes())
}

/// A regular-tile-group inter frame that parses to completion. Its
/// `refresh_frame_flags` IS parsed (`enable_short_refresh_frame_flags == 0` ->
/// f(NumRefFrames) read here), but as an INTER frame on a path the core parser does
/// not fully model, `frame_core_against_referenced_header` does not resolve it, so the
/// validator poisons all slots (the mask could touch any slot). Used to test honest
/// poisoning. The body is a best-effort inter header; the validator's core parse stops
/// before the flags, so the update is `PoisonAll` either way.
pub(in crate::validator::tests) fn ref_inter_tile_group() -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    // An INTER tile-group frame: the core parser's inter path is not fully modeled,
    // so the frame is unresolved and the validator poisons all slots.
    fb.bit(1); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(1)
    annex_b_obu(REGULAR_TILE_GROUP_HEADER, &fb.into_bytes())
}

pub(in crate::validator::tests) fn has_ref_slot_error(report: &ValidationReport) -> bool {
    report
        .errors()
        .any(|d| d.rule_id == "frame-header/show-existing-frame-invalid-slot")
}

/// A regular-tile-group INTER frame with the explicit reference map (the sequence
/// must set `explicit_ref_frame_map`), referencing `ref_slot` via `ref_frame_idx[0]`.
/// Built against the base inter sequence (NumRefFrames == 8 -> CeilLog2 == 3,
/// OrderHintBits == 1, monotonic output, enable_ref_frame_mvs == 0). The body parses
/// through the inter control region into the shared tail, so the core's
/// `inter.ref_frame_idx` is populated for the §6.17.2 slot-validity check.
pub(in crate::validator::tests) fn ref_inter_explicit_map(ref_slot: u32) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // frame_is_inter == 1 -> INTER_FRAME
    fb.bit(1); // immediate_output_frame (monotonic_output -> implicit forced 0, no bit)
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // signal_primary_ref_frame
    fb.bit(0); // disable_cross_frame_cdf_init (not TIP)
    fb.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8)
    fb.bit(1); // frame_explicit_ref_frame_map (explicit_ref_frame_map seq flag set)
    fb.f(1, 3); // num_total_refs == 1
    fb.f(ref_slot, 3); // ref_frame_idx[0] f(CeilLog2(8) == 3)
    // non-override, cur_mfh_id == 0 -> frame_size() default dims (no bits).
    // use_ref_frame_mvs: enable_ref_frame_mvs == 0 -> inferred 0 (no bit).
    // frame_opfl_refine_type: enable_opfl_refine != REFINE_AUTO -> no bits.
    fb.bit(0); // allow_screen_content_tools (SELECT) -> force_integer_mv = 0
    fb.bit(0); // allow_intrabc
    fb.bit(0); // use_qtr_precision_mv
    fb.bit(0); // allow_high_precision_mv -> HALF_PEL
    fb.bit(1); // is_filter_switchable -> SWITCHABLE (no interpolation_filter f(2))
    // motion modes: seq_frame_motion_modes_present_flag == 0 -> no bits.
    fb.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
    annex_b_obu(REGULAR_TILE_GROUP_HEADER, &fb.into_bytes())
}

pub(in crate::validator::tests) fn has_ref_frame_idx_error(report: &ValidationReport) -> bool {
    report
        .errors()
        .any(|d| d.rule_id == "frame-header/ref-frame-idx-invalid-slot")
}

pub(in crate::validator::tests) fn has_num_total_refs_error(report: &ValidationReport) -> bool {
    report
        .errors()
        .any(|d| d.rule_id == "frame-header/num-total-refs-out-of-range")
}

/// A regular-tile-group INTER frame (explicit reference map) coding `num_total_refs`
/// against a NumRefFrames == 2 sequence (`num_ref_frames_minus_1 == 1`), so
/// ActiveNumRefFrames = Min(REFS_PER_FRAME 7, 2) == 2 and an f(3) `num_total_refs`
/// of 3..=7 exceeds the §6.17.2 bound. Each `ref_frame_idx[i]` is f(CeilLog2(2) == 1).
/// Built against the base inter sequence (OrderHintBits == 1, monotonic output,
/// enable_ref_frame_mvs == 0); the body parses through the inter control region into
/// the shared tail, so the core's `inter.num_total_refs` is recorded for the check.
pub(in crate::validator::tests) fn ref_inter_num_total_refs(num_total_refs: u32) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // frame_is_inter == 1 -> INTER_FRAME
    fb.bit(1); // immediate_output_frame (monotonic_output -> implicit forced 0, no bit)
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // signal_primary_ref_frame
    fb.bit(0); // disable_cross_frame_cdf_init (not TIP)
    fb.f(0, 2); // refresh_frame_flags f(NumRefFrames == 2)
    fb.bit(1); // frame_explicit_ref_frame_map (explicit_ref_frame_map seq flag set)
    fb.f(num_total_refs, 3); // num_total_refs f(3)
    for _ in 0..num_total_refs {
        fb.f(0, 1); // ref_frame_idx[i] f(CeilLog2(2) == 1) -> slot 0
    }
    // non-override, cur_mfh_id == 0 -> frame_size() default dims (no bits).
    // use_ref_frame_mvs: enable_ref_frame_mvs == 0 -> inferred 0 (no bit). When
    // num_total_refs == 1 there is no tmvp read either way.
    // frame_opfl_refine_type: enable_opfl_refine != REFINE_AUTO -> no bits.
    fb.bit(0); // allow_screen_content_tools (SELECT) -> force_integer_mv = 0
    fb.bit(0); // allow_intrabc
    fb.bit(0); // use_qtr_precision_mv
    fb.bit(0); // allow_high_precision_mv -> HALF_PEL
    fb.bit(1); // is_filter_switchable -> SWITCHABLE (no interpolation_filter f(2))
    // motion modes: seq_frame_motion_modes_present_flag == 0 -> no bits.
    fb.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
    annex_b_obu(REGULAR_TILE_GROUP_HEADER, &fb.into_bytes())
}

#[test]
fn frame_header_num_total_refs_in_range_is_silent() {
    // §6.17.2 (mirror :4578-4579): num_total_refs <= ActiveNumRefFrames. NumRefFrames
    // == 2 -> ActiveNumRefFrames == 2; num_total_refs == 2 is exactly at the bound ->
    // silent.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        num_ref_frames_minus_1: 1, // NumRefFrames == 2 -> ActiveNumRefFrames == 2
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(ref_inter_num_total_refs(2)); // at the bound
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_num_total_refs_error(&report),
        "num_total_refs == ActiveNumRefFrames is conformant -> silent; report was: {report}"
    );
}

#[test]
fn frame_header_num_total_refs_out_of_range_fires() {
    // NumRefFrames == 2 -> ActiveNumRefFrames == 2; num_total_refs == 3 (> 2) violates
    // the §6.17.2 bound -> the diagnostic fires.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        num_ref_frames_minus_1: 1, // NumRefFrames == 2 -> ActiveNumRefFrames == 2
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(ref_inter_num_total_refs(3)); // exceeds the bound
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_num_total_refs_error(&report),
        "num_total_refs 3 > ActiveNumRefFrames 2 must fire the §6.17.2 check; \
         report was: {report}"
    );
}

#[test]
fn frame_header_truncated_inside_inter_control_fires_and_preserves_facts() {
    // Codex F2: an INTER frame whose payload ends INSIDE the modeled §5.18.2 inter
    // control region (here right after num_total_refs, before ref_frame_idx) must (1)
    // fire frame-header/truncated-frame-header (the core reports
    // StoppedInsideInterControl, a truncation in a fully-modeled region) AND (2) preserve
    // the facts parsed before the EOF — proven here by num_total_refs == 3 (> the
    // ActiveNumRefFrames == 2 bound) still firing frame-header/num-total-refs-out-of-range.
    // Pre-fix the inter EOF propagated UnexpectedEof out of parse_frame_header_core, the
    // validator's `.ok()` dropped both the truncation and every earlier fact, and the
    // frame validated clean (the PR #57/#59 regression class).
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        num_ref_frames_minus_1: 1, // NumRefFrames == 2 -> ActiveNumRefFrames == 2
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // frame_is_inter == 1 -> INTER_FRAME
    fb.bit(1); // immediate_output_frame (monotonic_output -> implicit forced 0, no bit)
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // signal_primary_ref_frame
    fb.bit(0); // disable_cross_frame_cdf_init (not TIP)
    fb.f(0, 2); // refresh_frame_flags f(NumRefFrames == 2)
    fb.bit(1); // frame_explicit_ref_frame_map
    fb.f(3, 3); // num_total_refs == 3 (> ActiveNumRefFrames 2) — last field before EOF
    // Truncate the payload to the byte that just contains num_total_refs, dropping
    // ref_frame_idx[i] so the parser EOFs while reading it.
    let keep_bytes = fb.bit_len().div_ceil(8);
    // Emit a few ref_frame_idx bits so into_bytes() pads a full final byte we then drop.
    fb.f(0, 1); // (dropped) the first ref_frame_idx[0] bit
    let mut payload = fb.into_bytes();
    payload.truncate(keep_bytes);
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &payload));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/truncated-frame-header"),
        "an EOF inside the modeled inter control region must fire \
         frame-header/truncated-frame-header; report was: {report}"
    );
    assert!(
        has_num_total_refs_error(&report),
        "the num_total_refs fact parsed before the EOF must survive and still fire the \
         §6.17.2 out-of-range check (facts preserved across truncation); report was: \
         {report}"
    );
}

pub(in crate::validator::tests) fn has_primary_ref_frame_error(report: &ValidationReport) -> bool {
    report
        .errors()
        .any(|d| d.rule_id == "frame-header/primary-ref-frame-out-of-range")
}

// --- § 6.17.2 RAS long_term_id_in_use(RefLongTermId[ref_frame_idx[i]]) ---

pub(in crate::validator::tests) fn has_ras_ref_long_term_error(report: &ValidationReport) -> bool {
    report
        .errors()
        .any(|d| d.rule_id == "frame-header/ras-ref-long-term-id-not-in-use")
}

/// A complete CLK KEY frame whose `long_term_id_plus_1` sets
/// `LongTermId = long_term_id_plus_1 - 1`, so the refreshed slot's modeled
/// `RefLongTermId` becomes that value. Built against a base `frame_core_seq` sequence
/// with `long_term_frame_id_bits == 4`. The body parses to completion so the validator
/// commits the §7.23 refresh (a CLK at FirstPictureInTU is a `ClkReset` — every slot is
/// cleared, then the refresh mask re-validates the named slots with these facts).
///
/// `max_mlayer_id` must match the active sequence header's value (§5.18.2 mirror :4429):
/// - `0`: the CLK refresh derives `allFrames` (no bits) and the §7.23 key `first` rule
///   leaves only slot 0 valid.
/// - `!= 0`: the CLK falls out of the `OBU_CLOSED_LOOP_KEY && max_mlayer_id == 0` arm and
///   reads `refresh_frame_flags f(NumRefFrames == 8)` explicitly (mirror :4443-4447); we
///   pass `1` so only slot 0 is refreshed (grounded with this `RefLongTermId`).
pub(in crate::validator::tests) fn clk_frame_long_term(
    long_term_id_plus_1: u32,
    max_mlayer_id: u32,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    // CLK -> KEY: long_term_id_plus_1 f(long_term_frame_id_bits == 4)
    fb.f(long_term_id_plus_1, 4);
    fb.bit(1); // immediate_output_frame
    fb.bit(1); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    if max_mlayer_id == 0 {
        // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits, mirror :4431-4433)
    } else {
        // mirror :4443-4447: the !enable_short_refresh else arm reads
        // refresh_frame_flags f(NumRefFrames == 8); 1 -> refresh slot 0 only.
        fb.f(1, 8); // refresh_frame_flags f(NumRefFrames == 8)
    }
    fb.f(15, 8); // frame_width_minus_1 f(8) -> 16 (== max_frame_width)
    fb.f(15, 8); // frame_height_minus_1 f(8) -> 16 (== max_frame_height)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0); // 16-wide frame -> no col-increment bit
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// A RAS frame (FrameType derived to SWITCH; explicit reference map) listing a single
/// `ref_long_term_id` and selecting slot 0 for every `ref_frame_idx[i]`. Built against a
/// base `frame_core_seq` sequence with `long_term_frame_id_bits == 4`,
/// `explicit_ref_frame_map == 1`, and `num_ref_frames` reference slots. A RAS forces
/// `frame_size_override_flag == 1` (SWITCH) so frame_size() reads explicit dims.
///
/// `max_mlayer_id` selects the §5.18.2 refresh arm (mirror :4493 vs :4507):
/// - `0`: the RAS refresh derives from RefValid/RefLongTermId, which the inter parser
///   cannot ground, so it stops with `InterStop::UnmodeledDerivation` BEFORE
///   `ref_frame_idx` (no `refresh_frame_flags` bits, parse never reaches the reference
///   region — the reachability boundary).
/// - `!= 0`: the RAS falls through to the SWITCH arm and reads
///   `refresh_frame_flags f(NumRefFrames)` explicitly, so the body parses through the
///   inter control region and `inter.ref_frame_idx` IS recorded for the §6.17.2
///   `long_term_id_in_use` check.
///
/// `num_total_refs` is coded as f(3); each `ref_frame_idx[i]` is f(CeilLog2(NumRefFrames))
/// and points at slot 0. (A `num_total_refs` above `Min(REFS_PER_FRAME, NumRefFrames)`
/// independently trips `frame-header/num-total-refs-out-of-range`, which doubles as a
/// proof the parse reached the reference region.)
pub(in crate::validator::tests) fn ras_frame_explicit_map(
    ref_long_term_id: u32,
    max_mlayer_id: u32,
    num_ref_frames: u32,
    num_total_refs: u32,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(0); // restricted_prediction_switch
    fb.f(1, 3); // num_key_ref_frames == 1
    fb.f(ref_long_term_id, 4); // ref_long_term_id[0] f(long_term_frame_id_bits == 4)
    fb.bit(1); // immediate_output_frame (RAS is not OLK)
    // monotonic_output_order_flag -> implicit_output_frame forced 0 (no bit)
    // SWITCH frame_type -> frame_size_override_flag forced 1 (no bit)
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    // FrameType derived to SWITCH -> primary_ref_frame = PRIMARY_REF_NONE and
    // disable_cross_frame_cdf_init = 0 are inferred (no bits, inter.rs :448-451).
    if max_mlayer_id == 0 {
        // refresh_frame_flags: RAS && max_mlayer_id == 0 -> derived (no bits, parse stops)
    } else {
        // mirror :4507-4509: FrameType == SWITCH -> refresh_frame_flags f(NumRefFrames).
        fb.f(0, num_ref_frames); // refresh_frame_flags f(NumRefFrames)
    }
    fb.bit(1); // frame_explicit_ref_frame_map (explicit_ref_frame_map seq flag set)
    fb.f(num_total_refs, 3); // num_total_refs f(3)
    for _ in 0..num_total_refs {
        fb.f(0, ceil_log2_u32(num_ref_frames)); // ref_frame_idx[i] -> slot 0
    }
    // SWITCH + frame_size_override_flag == 1: frame_size() reads explicit dims (not
    // frame_size_with_refs, since frame_type == Switch).
    fb.f(15, 8); // frame_width_minus_1 f(8) -> 16 (== max_frame_width)
    fb.f(15, 8); // frame_height_minus_1 f(8) -> 16 (== max_frame_height)
    annex_b_obu(RAS_HEADER, &fb.into_bytes())
}

#[test]
fn validator_ras_ref_long_term_silent_when_max_mlayer_zero_stops_before_ref_frame_idx() {
    // REACHABILITY BOUNDARY (named residual): for a RAS frame with max_mlayer_id == 0, the
    // §5.18.2 refresh_frame_flags derivation reads RefValid/RefLongTermId (mirror :4493),
    // which the inter parser cannot ground, so it stops with InterStop::UnmodeledDerivation
    // BEFORE reaching ref_frame_idx (inter.rs :524-526 / :482-485). `core.inter.ref_frame_idx`
    // is therefore EMPTY, so the §6.17.2 long_term_id_in_use check has nothing to evaluate
    // and stays silent — even though the RAS lists 3 while slot 0's modeled RefLongTermId is
    // 5. The check is reachable only for max_mlayer_id != 0 RAS frames (whose refresh is read
    // explicitly so the parse continues into the reference region). This proves the rule does
    // NOT false-positive on the unreachable single-embedded-layer case (zero false positives).
    let seq = FrameCoreSeq {
        long_term_frame_id_bits: 4,
        explicit_ref_frame_map: true,
        ..FrameCoreSeq::base() // max_mlayer_id == 0
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(clk_frame_long_term(6, 0)); // establishes slot 0 RefLongTermId 5
    data.extend(temporal_delimiter_obu());
    // max_mlayer_id == 0, NumRefFrames == 8, num_total_refs == 1: lists 3 (!= 5), but the
    // refresh derivation stops the parse before ref_frame_idx.
    data.extend(ras_frame_explicit_map(3, 0, 8, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_ras_ref_long_term_error(&report),
        "a RAS with max_mlayer_id == 0 stops before ref_frame_idx -> the §6.17.2 long-term \
         check is unreachable and must stay silent (no false positive); report was: {report}"
    );
}

#[test]
fn validator_silent_on_ras_ref_long_term_when_slot_unknown() {
    // The Unknown-invariant path: the parse REACHES ref_frame_idx (max_mlayer_id != 0, so
    // the RAS refresh derivation takes the explicit SWITCH arm instead of the unmodeled
    // RefValid/RefLongTermId derivation), but NO prior CLK establishes slot 0, so the §7.23
    // buffer leaves it Unknown. `slot_long_term_id` returns None for an Unknown slot, the
    // long-term check `continue`s, and the rule stays silent — even though the RAS lists 3.
    //
    // REACHABILITY GUARD (against future vacuity): NumRefFrames == 2 ->
    // ActiveNumRefFrames == Min(REFS_PER_FRAME 7, 2) == 2, and num_total_refs == 3 (> 2)
    // independently fires frame-header/num-total-refs-out-of-range — a fact decidable ONLY
    // after the parser reads num_total_refs in the reference region (immediately before
    // ref_frame_idx). Asserting that diagnostic DOES fire proves the parse advanced past the
    // refresh arm into the reference region (so the 3 recorded ref_frame_idx[i] = slot 0 ARE
    // present and the Unknown-slot branch is genuinely exercised), while the long-term rule
    // stays silent.
    let seq = FrameCoreSeq {
        long_term_frame_id_bits: 4,
        explicit_ref_frame_map: true,
        num_ref_frames_minus_1: 1, // NumRefFrames == 2 -> ActiveNumRefFrames == 2
        max_mlayer_id: 1,          // != 0 -> RAS refresh takes the explicit SWITCH arm
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    // No CLK: slot 0 is Unknown. NumRefFrames == 2, num_total_refs == 3 (> ActiveNumRefFrames).
    data.extend(ras_frame_explicit_map(3, 1, 2, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_num_total_refs_error(&report),
        "the RAS parse must reach the reference region (num_total_refs 3 > \
         ActiveNumRefFrames 2 fires the §6.17.2 bound) so ref_frame_idx is recorded and the \
         Unknown-slot path is genuinely exercised; report was: {report}"
    );
    assert!(
        !has_ras_ref_long_term_error(&report),
        "an Unknown slot cannot prove a §6.17.2 long-term violation -> silent; \
         report was: {report}"
    );
}

#[test]
fn validator_fires_on_ras_ref_long_term_when_slot_long_term_id_not_listed() {
    // The end-to-end FIRES path. A prior CLK grounds slot 0 with RefLongTermId 5
    // (long_term_id_plus_1 6 -> LongTermId 5), then a RAS frame (max_mlayer_id != 0, so the
    // parse reaches ref_frame_idx) selects slot 0 while listing only ref_long_term_id 3.
    // long_term_id_in_use(5) == 0 (5 is not in {3}), so the §6.17.2 conformance requirement
    // is violated and frame-header/ras-ref-long-term-id-not-in-use fires. NumRefFrames == 8
    // keeps num_total_refs == 1 in range (ActiveNumRefFrames == 7), isolating the long-term
    // rule.
    let seq = FrameCoreSeq {
        long_term_frame_id_bits: 4,
        explicit_ref_frame_map: true,
        max_mlayer_id: 1, // != 0 -> the RAS refresh takes the explicit SWITCH arm
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(clk_frame_long_term(6, 1)); // grounds slot 0 RefLongTermId 5
    data.extend(temporal_delimiter_obu());
    // NumRefFrames == 8, num_total_refs == 1, slot 0 selected; RAS lists 3 (!= 5).
    data.extend(ras_frame_explicit_map(3, 1, 8, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_ras_ref_long_term_error(&report),
        "a RAS selecting slot 0 (RefLongTermId 5) while listing only ref_long_term_id 3 \
         violates §6.17.2 long_term_id_in_use and must fire; report was: {report}"
    );
}

/// A regular-tile-group INTER frame (explicit reference map) that SIGNALS
/// `primary_ref_frame` (`signal_primary_ref_frame == 1`, f(3) value `primary_ref_frame`)
/// and codes `num_total_refs`, against the base inter sequence (NumRefFrames == 8 ->
/// CeilLog2 == 3 for each `ref_frame_idx`, OrderHintBits == 1, monotonic output,
/// enable_ref_frame_mvs == 0). The body parses through the inter control region into the
/// shared tail, so the core records both `signal_primary_ref_frame` / `primary_ref_frame`
/// and `num_total_refs` for the §6.17.2 range check.
pub(in crate::validator::tests) fn ref_inter_primary_ref(
    primary_ref_frame: u32,
    num_total_refs: u32,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // frame_is_inter == 1 -> INTER_FRAME
    fb.bit(1); // immediate_output_frame (monotonic_output -> implicit forced 0, no bit)
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(1); // signal_primary_ref_frame == 1 -> primary_ref_frame is present
    fb.bit(0); // disable_cross_frame_cdf_init (not TIP)
    fb.f(primary_ref_frame, 3); // primary_ref_frame f(3)
    fb.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8)
    fb.bit(1); // frame_explicit_ref_frame_map (explicit_ref_frame_map seq flag set)
    fb.f(num_total_refs, 3); // num_total_refs f(3)
    for _ in 0..num_total_refs {
        fb.f(0, 3); // ref_frame_idx[i] f(CeilLog2(8) == 3) -> slot 0
    }
    // non-override, cur_mfh_id == 0 -> frame_size() default dims (no bits).
    // use_ref_frame_mvs: enable_ref_frame_mvs == 0 -> inferred 0 (no bit).
    // frame_opfl_refine_type: enable_opfl_refine != REFINE_AUTO -> no bits.
    fb.bit(0); // allow_screen_content_tools (SELECT) -> force_integer_mv = 0
    fb.bit(0); // allow_intrabc
    fb.bit(0); // use_qtr_precision_mv
    fb.bit(0); // allow_high_precision_mv -> HALF_PEL
    fb.bit(1); // is_filter_switchable -> SWITCHABLE (no interpolation_filter f(2))
    // motion modes: seq_frame_motion_modes_present_flag == 0 -> no bits.
    fb.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
    annex_b_obu(REGULAR_TILE_GROUP_HEADER, &fb.into_bytes())
}

#[test]
fn frame_header_primary_ref_frame_in_range_is_silent() {
    // §6.17.2 (mirror :4500-4502): a signaled primary_ref_frame must be PRIMARY_REF_NONE
    // or < NumTotalRefs. num_total_refs == 2, primary_ref_frame == 1 (< 2) -> in range,
    // silent.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(ref_inter_primary_ref(1, 2)); // primary_ref_frame 1 < num_total_refs 2
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_primary_ref_frame_error(&report),
        "primary_ref_frame 1 < NumTotalRefs 2 is conformant -> silent; report was: {report}"
    );
}

#[test]
fn frame_header_primary_ref_frame_out_of_range_fires() {
    // num_total_refs == 1, primary_ref_frame == 5 (signaled, not PRIMARY_REF_NONE, not
    // < 1) -> the §6.17.2 range check fires.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(ref_inter_primary_ref(5, 1)); // primary_ref_frame 5 >= num_total_refs 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_primary_ref_frame_error(&report),
        "a signaled primary_ref_frame 5 that is neither PRIMARY_REF_NONE nor < NumTotalRefs \
         1 must fire the §6.17.2 range check; report was: {report}"
    );
}

/// A regular-tile-group INTER frame (explicit reference map, `enable_bru` sequence)
/// that codes `num_total_refs`, the `ref_frame_idx` loop, then the §5.18.2 BRU triple
/// (`use_bru == 1`, `bru_ref` f(CeilLog2(num_total_refs)), `bru_inactive == 0`) and
/// completes the control region into the shared tail, for the §6.17.2 BRU checks.
pub(in crate::validator::tests) fn ref_inter_bru(
    immediate_output: bool,
    num_total_refs: u32,
    bru_ref: u32,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // frame_is_inter == 1 -> INTER_FRAME
    fb.bit(u8::from(immediate_output)); // immediate_output_frame (monotonic -> no implicit bit)
    fb.bit(0); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // signal_primary_ref_frame == 0 -> PRIMARY_REF_CHOOSE (no f(3))
    fb.bit(0); // disable_cross_frame_cdf_init (not TIP)
    fb.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8)
    fb.bit(1); // frame_explicit_ref_frame_map
    fb.f(num_total_refs, 3); // num_total_refs f(3)
    for _ in 0..num_total_refs {
        fb.f(0, 3); // ref_frame_idx[i] f(CeilLog2(8) == 3) -> slot 0
    }
    // non-override, cur_mfh_id == 0 -> frame_size() default dims (no bits).
    // mirror :4653-4669: the BRU triple (enable_bru sequence, INTER, not TIP/bridge).
    fb.bit(1); // use_bru == 1
    let n = 32 - (num_total_refs - 1).leading_zeros();
    fb.f(bru_ref, n); // bru_ref f(CeilLog2(num_total_refs))
    fb.bit(0); // bru_inactive == 0 -> no early return
    // use_ref_frame_mvs: enable_ref_frame_mvs == 0 -> inferred 0 (no bit).
    // frame_opfl_refine_type: enable_opfl_refine != REFINE_AUTO -> no bits.
    fb.bit(0); // allow_screen_content_tools (SELECT) -> force_integer_mv = 0
    fb.bit(0); // allow_intrabc
    fb.bit(0); // use_qtr_precision_mv
    fb.bit(0); // allow_high_precision_mv -> HALF_PEL
    fb.bit(1); // is_filter_switchable -> SWITCHABLE (no interpolation_filter f(2))
    // motion modes: seq_frame_motion_modes_present_flag == 0 -> no bits.
    fb.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
    annex_b_obu(REGULAR_TILE_GROUP_HEADER, &fb.into_bytes())
}

#[test]
fn frame_header_bru_ref_out_of_range_fires() {
    // §6.17.2 (mirror :4592): when use_bru == 1, bru_ref must be less than
    // NumTotalRefs. num_total_refs == 3 -> bru_ref f(CeilLog2(3) == 2) can code 3,
    // which violates the bound.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        enable_bru: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(ref_inter_bru(true, 3, 3));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/bru-ref-out-of-range"),
        "bru_ref 3 >= NumTotalRefs 3 must fire the §6.17.2 bound; report was: {report}"
    );
}

#[test]
fn frame_header_bru_in_range_with_output_is_silent() {
    // bru_ref 2 < NumTotalRefs 3 and immediate_output_frame == 1: both §6.17.2 BRU
    // clauses hold -> neither BRU diagnostic fires.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        enable_bru: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(ref_inter_bru(true, 3, 2));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/bru-ref-out-of-range"
                || d.rule_id == "frame-header/bru-without-immediate-output"),
        "a conformant BRU frame must not fire the §6.17.2 BRU checks; report was: {report}"
    );
}

#[test]
fn frame_header_bru_without_immediate_output_fires() {
    // §6.17.2 (mirror :4591): use_bru == 1 requires immediate_output_frame == 1.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        enable_bru: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(ref_inter_bru(false, 3, 2));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/bru-without-immediate-output"),
        "use_bru == 1 with immediate_output_frame == 0 must fire the §6.17.2 check; \
         report was: {report}"
    );
}

#[test]
fn ref_state_inter_ref_frame_idx_proven_invalid_fires() {
    // §7.23 / §5.18.2 / §6.17.2: a CLK at FirstPictureInTU resets RefValid over
    // 0..NumRefFrames and its allFrames refresh re-validates only slot 0. A later INTER
    // frame whose explicit ref_frame_idx[0] names slot 3 -> the buffer PROVES
    // RefValid[3] == 0, so the inter ref-idx slot-validity diagnostic fires.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(clk_frame_decidable(true, true)); // CLK: reset + allFrames refresh
    data.extend(ref_inter_explicit_map(3)); // INTER referencing proven-invalid slot 3
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_ref_frame_idx_error(&report),
        "an inter frame whose ref_frame_idx names a CLK-invalidated, never-refreshed slot \
         must fire the inter ref-idx slot-validity check; report was: {report}"
    );
}

#[test]
fn ref_state_inter_ref_frame_idx_valid_slot_is_silent() {
    // The same CLK leaves slot 0 valid. An INTER frame referencing slot 0 via
    // ref_frame_idx[0] is conformant -> silent.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(clk_frame_decidable(true, true));
    data.extend(ref_inter_explicit_map(0)); // INTER referencing the valid slot 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_ref_frame_idx_error(&report),
        "an inter frame referencing the valid slot 0 must be silent; report was: {report}"
    );
}

#[test]
fn ref_state_inter_ref_frame_idx_poisoned_slot_drops_to_silence() {
    // A mid-stream join (no observed CLK reset) leaves every slot Unknown. An INTER
    // frame referencing slot 3 -> Unknown (not proven invalid) -> silent.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(ref_inter_explicit_map(3)); // first frame, all slots Unknown
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_ref_frame_idx_error(&report),
        "an inter frame against an unestablished (Unknown) buffer must drop to silence; \
         report was: {report}"
    );
}

#[test]
fn ref_state_sef_proven_invalid_slot_fires() {
    // §7.23 / §5.18.2 / §6.17.2: a CLK at FirstPictureInTU resets RefValid[i] = 0 over
    // 0..NumRefFrames, then its allFrames refresh (max_mlayer_id == 0) re-validates only
    // slot 0 (the key-frame `first` rule). A later SEF referencing slot 3 -> the buffer
    // PROVES RefValid[3] == 0, so the slot-validity diagnostic fires.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // CLK: reset + allFrames refresh
    data.extend(ref_sef(3)); // SEF displaying proven-invalid slot 3
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_ref_slot_error(&report),
        "a SEF referencing a CLK-invalidated, never-refreshed slot must fire the \
         show-existing-frame slot-validity check; report was: {report}"
    );
}

#[test]
fn ref_state_sef_valid_slot_is_silent() {
    // The same CLK leaves slot 0 valid (lowest refreshed slot of the allFrames mask
    // under the key-frame `first` rule). A SEF referencing slot 0 is conformant ->
    // silent.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true));
    data.extend(ref_sef(0)); // SEF displaying the valid slot 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_ref_slot_error(&report),
        "a SEF referencing the valid slot 0 must be silent; report was: {report}"
    );
}

#[test]
fn ref_state_sef_poisoned_slot_drops_to_silence() {
    // A mid-stream join (no observed CLK reset) leaves every slot Unknown. The
    // tracker only fires when it PROVES RefValid == 0; an Unknown slot drops to
    // silence (the Unknown invariant). A SEF as the first frame after just a TD + seq
    // header references slot 3 -> Unknown -> silent.
    let mut data = seg_td_and_seq();
    data.extend(ref_sef(3)); // first frame, all slots Unknown
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_ref_slot_error(&report),
        "a SEF against an unestablished (poisoned/Unknown) buffer must drop to silence; \
         report was: {report}"
    );
}

#[test]
fn ref_state_inter_frame_poisons_then_sef_is_silent() {
    // After a CLK establishes slot 0 valid, an INTER tile-group frame whose refresh
    // mask the validator cannot ground poisons ALL slots. A subsequent SEF referencing
    // slot 0 -> now Unknown (not proven invalid) -> silent. This proves honest
    // poisoning: the previously-valid slot is no longer judged once poisoned.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // slot 0 valid
    data.extend(ref_inter_tile_group()); // poisons all slots
    data.extend(ref_sef(0)); // slot 0 now Unknown -> silent
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_ref_slot_error(&report),
        "an unparsed inter refresh mask must poison the buffer, so a later SEF against a \
         once-valid slot drops to silence; report was: {report}"
    );
    // And the inter frame must not itself spuriously fire the slot check.
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "frame-header/show-existing-frame-invalid-slot"),
    );
}

#[test]
fn ref_state_sef_invalid_then_inter_then_sef_silent_proves_poison_lifetime() {
    // Ordering: CLK (slot 0 valid, slots 1.. proven invalid) -> SEF(3) fires ->
    // inter poisons -> SEF(3) now silent (poisoned, no longer PROVEN invalid). The
    // poison transition is observable: the SAME reference flips from firing to silent.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true));
    data.extend(ref_sef(3)); // proven invalid -> fires
    let report_before = Validator::new(false).validate_bytes(&data);
    assert!(
        has_ref_slot_error(&report_before),
        "report: {report_before}"
    );

    let mut data2 = seg_td_and_seq();
    data2.extend(clk_frame_decidable(true, true));
    data2.extend(ref_inter_tile_group()); // poison all slots
    data2.extend(ref_sef(3)); // now Unknown -> silent
    let report_after = Validator::new(false).validate_bytes(&data2);
    assert!(
        !has_ref_slot_error(&report_after),
        "after poisoning, the same slot-3 reference must drop to silence; \
         report was: {report_after}"
    );
}

#[test]
fn ref_state_clk_reset_across_cvs_revalidates_only_slot_zero() {
    // Two CVS epochs: each CLK resets and re-validates only slot 0. A SEF after the
    // second CLK referencing slot 5 must fire (proven invalid), and referencing slot 0
    // must be silent — confirming the CLK reset re-runs per CVS.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // CVS 1
    data.extend(temporal_delimiter_obu());
    data.extend(clk_frame_decidable(true, true)); // CVS 2 (new CLK -> reset again)
    data.extend(ref_sef(5)); // proven invalid in CVS 2
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_ref_slot_error(&report),
        "a SEF after the second CLK referencing a proven-invalid slot must fire; \
         report was: {report}"
    );
}

#[test]
fn ref_state_unsupported_inter_frame_poisons_not_refreshes() {
    // §7.23 staging gate (F2): an INTER frame whose core RESOLVES against its active
    // sequence header records `refresh_frame_flags` and dims on the core, but its parse
    // stops at `UnsupportedUntilFeature` past the prefix (the inter reference-control
    // region is not parsed to completion this phase). Staging a normal §7.23 Refresh
    // from such a frame asserts buffer facts the validator has not earned — the frame's
    // decodability is unestablished and its slot effect could be wrong both ways — so
    // `derive_ref_update` must POISON ALL slots, not refresh.
    //
    // Observable consequence (the wrong vs. right behavior diverges here):
    //   1. CLK -> slot 0 valid, slots 1.. PROVEN invalid (key-frame `first` rule).
    //   2. INTER frame, explicit map, `refresh_frame_flags == 0` (refreshes no slot),
    //      core resolves, status `UnsupportedUntilFeature`.
    //   3. SEF displaying slot 5.
    // Pre-fix: the inter frame stages `Refresh { mask = 0 }`, which leaves the
    //   already-PROVEN-invalid slot 5 untouched -> still ProvenInvalid -> the SEF(5)
    //   slot-validity check FIRES. But that is a FALSE POSITIVE: after an inter frame
    //   the validator could not fully parse, the buffer state is genuinely unknown, so
    //   it cannot prove slot 5 is still invalid.
    // Post-fix: the unsupported inter frame POISONS all slots -> slot 5 Unknown -> the
    //   SEF(5) check correctly drops to silence (the zero-false-positive rule).
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    data.extend(clk_frame_decidable(true, true)); // CLK: slot 0 valid, 1.. proven invalid
    data.extend(ref_inter_explicit_map(0)); // resolving INTER, refresh mask 0, UNSUPPORTED
    data.extend(ref_sef(5)); // SEF displaying slot 5
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_ref_slot_error(&report),
        "an UNSUPPORTED inter frame (parse stopped past the prefix) must POISON the §7.23 \
         buffer, not stage a normal Refresh from its recorded mask; after poisoning, the \
         SEF(5) slot-validity check must drop to silence (it can no longer prove slot 5 \
         invalid). Pre-fix the inter frame staged a Refresh that left slot 5 ProvenInvalid \
         and the SEF check fired a FALSE POSITIVE; report was: {report}"
    );
}

#[test]
fn ref_state_completed_intra_still_refreshes_after_gate() {
    // §7.23 staging gate (F2), the positive direction: a COMPLETED intra CLK
    // (`IntraHeaderComplete`) still stages its grounded §7.23 ClkReset + allFrames
    // refresh — the gate only blocks incomplete / unsupported / truncated parses, not
    // grounded complete ones. The CLK re-validates slot 0; a SEF(5) is still PROVEN
    // invalid (the reset cleared it and the allFrames `first` rule re-validated only
    // slot 0), so the slot-validity check fires exactly as before the gate.
    let mut data = seg_td_and_seq();
    data.extend(clk_frame_decidable(true, true)); // COMPLETED CLK -> reset + refresh slot 0
    data.extend(ref_sef(5)); // proven invalid -> must still fire (the refresh is grounded)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_ref_slot_error(&report),
        "a completed intra CLK must still stage its grounded ClkReset/refresh through the \
         gate, so a SEF(5) on the proven-invalid slot fires; report was: {report}"
    );
    // And referencing the re-validated slot 0 is silent, confirming the refresh landed.
    let mut data0 = seg_td_and_seq();
    data0.extend(clk_frame_decidable(true, true));
    data0.extend(ref_sef(0));
    let report0 = Validator::new(false).validate_bytes(&data0);
    assert!(
        !has_ref_slot_error(&report0),
        "the completed CLK's refresh must re-validate slot 0 (SEF(0) silent); \
         report was: {report0}"
    );
}

/// A COMPLETED override CLK key frame whose explicit frame size is `width`x`height`
/// (read as f(frame_width_bits == 8) each), against the base inter sequence. The CLK
/// reset + allFrames refresh (max_mlayer_id == 0, key-frame `first` rule) re-validates
/// slot 0 with these stored dims (`RefFrameWidth/Height[0]`), which a later
/// `frame_size_with_refs()` inter frame copies. A nonzero `base_q_idx` and the complete
/// §5.18.2 intra tail land the parse on `IntraHeaderComplete`, so `derive_ref_update`
/// stages a grounded §7.23 update (not a poison).
pub(in crate::validator::tests) fn clk_frame_override_size(width: u32, height: u32) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // immediate_output_frame
    fb.bit(1); // frame_size_override_flag
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    // refresh: CLK + max_mlayer_id == 0 -> allFrames (no bits)
    fb.f(width - 1, 8); // frame_width_minus_1 f(frame_width_bits == 8)
    fb.f(height - 1, 8); // frame_height_minus_1 f(frame_height_bits == 8)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    // A 256-wide frame: sbCols == 4, so tile_info() reads one column increment bit. A
    // 16-wide frame reads none; this helper is used at 256 width.
    intra_structure_tail(&mut fb, 1);
    fb.bit(0); // tx_mode_select = 0
    fb.f(0, 2); // reduced_tx_set = 0
    annex_b_obu(CLK_HEADER, &fb.into_bytes())
}

/// A regular-tile-group INTER frame (explicit reference map) on the OVERRIDE path that
/// derives its frame size via `frame_size_with_refs()` (§5.18.4.3): `found_ref == 1` on
/// the first ref, so FrameWidth/Height are copied from `RefFrameWidth/Height[ref_slot]`
/// — no explicit width/height bits are read. Built against the base inter sequence
/// (NumRefFrames == 8 -> CeilLog2 == 3, OrderHintBits == 1, monotonic output,
/// enable_ref_frame_mvs == 0). The parse records `core.frame_size` from the referenced
/// slot's stored dims, which the §6.17.4.1 frame-size-exceeds-sequence-max check reads.
pub(in crate::validator::tests) fn inter_frame_size_with_refs(ref_slot: u32) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header
    fb.bit(1); // frame_is_inter == 1 -> INTER_FRAME
    fb.bit(1); // immediate_output_frame (monotonic_output -> implicit forced 0, no bit)
    fb.bit(1); // frame_size_override_flag == 1 -> frame_size_with_refs() on the inter path
    fb.f(0, 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // signal_primary_ref_frame
    fb.bit(0); // disable_cross_frame_cdf_init (not TIP)
    fb.f(0, 8); // refresh_frame_flags f(NumRefFrames == 8)
    fb.bit(1); // frame_explicit_ref_frame_map (explicit_ref_frame_map seq flag set)
    fb.f(1, 3); // num_total_refs == 1
    fb.f(ref_slot, 3); // ref_frame_idx[0] f(CeilLog2(8) == 3)
    // frame_size_with_refs(): override && !SWITCH -> found_ref f(1) per ref until a hit;
    // found_ref == 1 on ref 0 copies RefFrameWidth/Height[ref_slot] (no width/height bits).
    fb.bit(1); // found_ref == 1
    // use_ref_frame_mvs: enable_ref_frame_mvs == 0 -> inferred 0 (no bit).
    // frame_opfl_refine_type: enable_opfl_refine != REFINE_AUTO -> no bits.
    fb.bit(0); // allow_screen_content_tools (SELECT) -> force_integer_mv = 0
    fb.bit(0); // allow_intrabc
    fb.bit(0); // use_qtr_precision_mv
    fb.bit(0); // allow_high_precision_mv -> HALF_PEL
    fb.bit(1); // is_filter_switchable -> SWITCHABLE (no interpolation_filter f(2))
    // motion modes: seq_frame_motion_modes_present_flag == 0 -> no bits.
    fb.bit(0); // disable_cdf_update f(1) (mirror :5041), just before the shared tail.
    annex_b_obu(REGULAR_TILE_GROUP_HEADER, &fb.into_bytes())
}

#[test]
fn ref_state_inter_frame_size_with_refs_sees_prior_frame_refresh() {
    // REGRESSION (codex F1): the §6.17 frame-size diagnostics emitted from
    // `frame_header_core_checks` (in `observe_frame_bearing_obu`) must see the prior
    // frame's COMMITTED §7.23 reference-state update, not a stale snapshot. The bug: the
    // reference-buffer snapshot threaded into the inter parser is taken in
    // `observe_frame_bearing_obu`, which runs BEFORE `observe_reference_state` commits
    // the previous frame's deferred §7.23 update (the PR #62 deferred-commit pattern
    // commits at the next OpensNewUnit boundary). So a frame opening a new coded frame
    // immediately after the previous frame refreshed a slot snapshots STALE state.
    //
    // Scenario:
    //   1. CLK key frame, override size 256x8 (out of range), refreshes slot 0 valid
    //      with stored dims 256x8 (allFrames + key-frame `first` rule). Its own size
    //      fires §6.17.4.1 at the CLK's offset.
    //   2. INTER frame, override path, `frame_size_with_refs()` with `found_ref == 1` on
    //      `ref_frame_idx[0] == 0` -> FrameWidth/Height = RefFrameWidth/Height[0] = 256x8,
    //      which exceeds the sequence max 16x16 (§6.17.4.1) at the INTER frame's offset.
    //
    // Post-fix the slot-0 refresh is committed before the inter parse's snapshot, so
    // `frame_size_with_refs()` grounds 256x8 and the §6.17.4.1 check fires at the inter
    // frame's offset. Pre-fix the snapshot is stale (slot 0 still Unknown), so
    // `frame_size_with_refs()` poisons, `core.frame_size` is None, and the §6.17.4.1
    // check is SILENTLY skipped for the inter frame.
    let seq = FrameCoreSeq {
        explicit_ref_frame_map: true,
        ..FrameCoreSeq::base()
    };
    let mut data = td_and_frame_core_seq(seq);
    let clk = clk_frame_override_size(256, 8);
    // The inter frame's diagnostic anchors at its OBU header, one Annex B leb128
    // size-prefix byte past the end of the preceding OBUs (the CLK).
    let inter_obu_offset = (data.len() + clk.len()) as u64 + 1;
    data.extend(clk); // refreshes slot 0 with dims 256x8
    data.extend(inter_frame_size_with_refs(0)); // copies slot-0 dims via frame_size_with_refs
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| {
            d.rule_id == "frame-header/frame-size-exceeds-sequence-max"
                && d.byte_offset.map(|offset| offset.get()) == Some(inter_obu_offset)
        }),
        "the inter frame's frame_size_with_refs() must see the prior frame's committed \
         slot-0 refresh (dims 256x8 > max 16x16) and fire §6.17.4.1 at the inter frame's \
         offset {inter_obu_offset}; pre-fix the stale snapshot left slot 0 Unknown so the \
         size poisoned and the check was skipped. report was: {report}"
    );
}
