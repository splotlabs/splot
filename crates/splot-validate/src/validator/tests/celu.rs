// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

// --- § 7.3.6 / § 7.3.7 / § 7.4.6 coded-extended-layer-unit + DOH integration ---

/// A full intra output CLK frame at `xlayer` (mlayer 0) referencing seq 0, with a
/// chosen `order_hint` (OrderHintBits == 1, so order_hint is f(1)). The body reaches
/// `intra_structure_tail`, so the core parser settles `immediate_output_frame == 1`
/// (output) and reads `order_hint` — giving the CELU tracker a decidable output class
/// and OrderHint. Built like `clk_frame_decidable` but layered.
pub(in crate::validator::tests) fn celu_output_clk_at(xlayer: u8, order_hint: u8) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(0); // seq_header_id_in_frame_header -> seq 0
    fb.bit(1); // immediate_output_frame == 1 (output)
    fb.bit(0); // frame_size_override_flag
    fb.f(u32::from(order_hint), 1); // order_hint f(OrderHintBits == 1)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    annex_b_obu_with_header(&layer_obu_header(4, 0, 0, xlayer), &fb.into_bytes())
}

#[test]
fn celu_same_celu_output_order_hint_mismatch_is_flagged() {
    // Two output CLK frames in one coded extended layer unit (xlayer 0, mlayer 0) with
    // different OrderHint values violate the § 7.3.6 same-OrderHint rule. The order_hint
    // is read from the real frame-header core parse.
    let mut data = seg_td_and_seq();
    data.extend(celu_output_clk_at(0, 0)); // OrderHint 0
    data.extend(celu_output_clk_at(0, 1)); // OrderHint 1 -> mismatch
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/output-order-hint-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn celu_same_celu_output_order_hint_agreement_is_silent() {
    let mut data = seg_td_and_seq();
    data.extend(celu_output_clk_at(0, 1));
    data.extend(celu_output_clk_at(0, 1)); // same OrderHint
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/output-order-hint-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn celu_doh_cross_celu_order_hint_mismatch_under_msdo_flag_is_flagged() {
    // A temporal unit with an MSDO whose multistream_doh_constraint_flag == 1, then two
    // CELUs (xlayer 0 and xlayer 1) whose output CLK frames carry different OrderHints —
    // the § 7.3.7 / § 7.4.6 cross-CELU DOH OrderHint constraint fires.
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(annex_b_obu(
        0x04,
        &frame_core_seq_payload(FrameCoreSeq::base()),
    ));
    data.extend(celu_output_clk_at(0, 0)); // CELU xlayer 0, OrderHint 0
    data.extend(celu_output_clk_at(1, 1)); // CELU xlayer 1, OrderHint 1 -> cross-CELU mismatch
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-mismatch"),
        "report was: {report}"
    );
}

#[test]
fn celu_doh_cross_celu_order_hint_mismatch_without_flag_is_silent() {
    // The same cross-CELU OrderHint disagreement with multistream_doh_constraint_flag == 0
    // (and no activated global LCR) is conforming — the DOH constraint is flag-gated.
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(annex_b_obu(
        0x04,
        &frame_core_seq_payload(FrameCoreSeq::base()),
    ));
    data.extend(celu_output_clk_at(0, 0));
    data.extend(celu_output_clk_at(1, 1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-mismatch"),
        "the DOH OrderHint check must stay silent with the flag off; report was: {report}"
    );
}

/// A full intra output CLK at `xlayer` (mlayer 0) referencing `seq_header_id`, whose
/// active sequence header has `OrderHintBits == order_hint_bits`, carrying `order_hint`
/// as `f(order_hint_bits)`. Like [`celu_output_clk_at`] but parameterised so a frame can
/// reference a sequence header with a different OrderHintBits (Finding B) or an absent
/// sequence header (Finding A).
pub(in crate::validator::tests) fn celu_output_clk_ref(
    xlayer: u8,
    seq_header_id: u32,
    order_hint_bits: u32,
    order_hint: u32,
) -> Vec<u8> {
    celu_clk_ref(xlayer, seq_header_id, order_hint_bits, order_hint, true)
}

/// [`celu_output_clk_ref`] with a chosen `immediate_output_frame`. With the sequence's
/// `monotonic_output_order_flag == 1`, `immediate_output == false` settles a decided
/// NON-output class (no `implicit_output_frame` bit is read).
pub(in crate::validator::tests) fn celu_clk_ref(
    xlayer: u8,
    seq_header_id: u32,
    order_hint_bits: u32,
    order_hint: u32,
    immediate_output: bool,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(seq_header_id); // seq_header_id_in_frame_header
    fb.bit(u8::from(immediate_output)); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag
    fb.f(order_hint, order_hint_bits); // order_hint f(OrderHintBits)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    annex_b_obu_with_header(&layer_obu_header(4, 0, 0, xlayer), &fb.into_bytes())
}

/// A `frame_core_seq` sequence header on `xlayer` with a chosen `seq_id` and
/// `OrderHintBits == order_hint_bits` (else the base config), for the cross-OrderHintBits
/// DOH tests.
pub(in crate::validator::tests) fn celu_seq_header_obu(
    xlayer: u8,
    seq_id: u32,
    order_hint_bits: u32,
) -> Vec<u8> {
    let seq = FrameCoreSeq {
        seq_id,
        order_hint_bits_minus_1: order_hint_bits - 1,
        ..FrameCoreSeq::base()
    };
    let payload = frame_core_seq_payload(seq);
    if xlayer == 0 {
        annex_b_obu(0x04, &payload)
    } else {
        annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
    }
}

#[test]
fn celu_doh_cross_celu_order_hint_mismatch_with_different_bits_is_silent() {
    // Finding B (end-to-end): two CELUs under the DOH flag whose output CLK frames carry
    // differing `order_hint` LSBs, but whose active sequence headers declare DIFFERENT
    // (known) OrderHintBits (1 vs 2). The cross-CELU OrderHint comparison is an LSB proxy
    // and is UNSOUND across different bit widths (equal decoded OrderHints can encode to
    // different-width LSBs), so it must DROP — only celu/doh-order-hint-bits-mismatch
    // fires. Pre-fix the OrderHint comparison fired off the raw LSBs (a false positive).
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(celu_seq_header_obu(0, 0, 1)); // xlayer 0 active header: OrderHintBits 1
    data.extend(celu_seq_header_obu(1, 1, 2)); // xlayer 1 active header: OrderHintBits 2
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // CELU xlayer 0, LSB 0
    data.extend(celu_output_clk_ref(1, 1, 2, 1)); // CELU xlayer 1, LSB 1 (differs)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-bits-mismatch"),
        "differing OrderHintBits across the temporal unit must fire the bits rule; \
         report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-mismatch"),
        "the cross-CELU OrderHint comparison must DROP when OrderHintBits differ (the LSB \
         proxy is unsound across bit widths); report was: {report}"
    );
}

#[test]
fn celu_doh_cross_celu_order_hint_mismatch_with_equal_bits_is_flagged() {
    // Finding B (end-to-end): the sound case — two CELUs under the DOH flag with EQUAL,
    // known OrderHintBits (both 1) and differing `order_hint` LSBs. Same-width differing
    // LSBs imply different decoded OrderHints, so the cross-CELU OrderHint comparison is a
    // sound under-approximation and fires (the bits rule stays silent).
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(celu_seq_header_obu(0, 0, 1)); // xlayer 0: OrderHintBits 1
    data.extend(celu_seq_header_obu(1, 1, 1)); // xlayer 1: OrderHintBits 1 (equal)
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // LSB 0
    data.extend(celu_output_clk_ref(1, 1, 1, 1)); // LSB 1 (differs)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-mismatch"),
        "equal known OrderHintBits with differing LSBs must fire the OrderHint rule; \
         report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-bits-mismatch"),
        "equal OrderHintBits must not fire the bits rule; report was: {report}"
    );
}

// --- Finding A: stale-activation misparse guard (§ 5.18.2 / § 7.3.6) ---------------

#[test]
fn celu_frame_referencing_absent_seq_header_is_unknown_not_misparsed() {
    // Finding A: a frame whose referenced sequence header is unavailable in-band must NOT
    // be parsed against a STALE earlier activation — its sequence-header-dependent fields
    // (output class, order_hint) would misparse and could fire celu/* judgments off
    // garbage (a false positive). Here TU1 activates seq 0 (OrderHintBits 1) for xlayer 0.
    // TU2 carries a single CLK referencing the ABSENT seq 1; built so that parsing it
    // against the stale seq 0 settles a DECIDED non-output class — which would fire
    // celu/missing-output-frame-unit (a decided non-output unit, no output unit).
    // Post-fix the referenced-header guard returns Unknown (the parsed
    // seq_header_id_in_frame_header != the parsed-against id), so the output class is
    // undecidable and missing-output-frame-unit DROPS.
    let mut data = temporal_delimiter_obu();
    data.extend(celu_seq_header_obu(0, 0, 1)); // seq 0, OrderHintBits 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // TU1: activates seq 0 for xlayer 0
    data.extend(temporal_delimiter_obu()); // TU2
    // CLK referencing the ABSENT seq 1, immediate_output == 0 -> parsed against the stale
    // seq 0 it is a DECIDED non-output frame; correctly it is Unknown.
    data.extend(celu_clk_ref(0, 1, 1, 0, false));
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    // The TU2 CELU's only frame must route to Unknown, dropping the output-class judgment.
    // (The unavailable seq-1 reference still emits its own hls/* diagnostic — orthogonal.)
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/missing-output-frame-unit"),
        "a frame referencing an absent sequence header must route to Unknown rather than \
         misparse against the stale activation; report was: {report}"
    );
}

#[test]
fn celu_frame_referencing_inband_seq_header_is_decided() {
    // Finding A (negative): normal in-band resolution is unchanged. TU2's CLK references
    // seq 0, which IS available and is the active header it parses against, so the guard
    // passes and the decided non-output class still fires celu/missing-output-frame-unit
    // (a single decided non-output unit, no output unit) — proving the guard does not
    // over-suppress the in-band case.
    let mut data = temporal_delimiter_obu();
    data.extend(celu_seq_header_obu(0, 0, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // TU1: activates seq 0
    data.extend(temporal_delimiter_obu()); // TU2
    data.extend(celu_clk_ref(0, 0, 1, 0, false)); // references the available seq 0
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/missing-output-frame-unit"),
        "an in-band-resolved non-output frame must stay decided and fire missing-output; \
         report was: {report}"
    );
}

// --- F4: DOH OrderHintBits stale-guard (§ 7.3.7) -----------------------------------

#[test]
fn celu_doh_order_hint_bits_absent_ref_frame_contributes_no_bits() {
    // F4: under the DOH flag, the per-frame OrderHintBits contribution must be gated on the
    // SAME resolution decision the output-class derivation uses — only contribute bits when
    // the frame's parsed prefix resolved to the active header. The two active headers are
    // established in SEPARATE temporal units so no single temporal unit legitimately carries
    // two different OrderHintBits: TU1 activates seq 0 (OrderHintBits 1) for xlayer 0; TU2
    // activates seq 1 (OrderHintBits 2) for xlayer 1. An MSDO with
    // multistream_doh_constraint_flag == 1 opens a persisting CMVS. TU3 (CMVS still open) is
    // the test temporal unit: xlayer 0's frame resolves (contributes bits 1); xlayer 1's
    // frame references the ABSENT seq 5, so it does NOT resolve to its stale active header
    // (seq 1, bits 2). Post-fix it contributes NO bits (None -> bits_undecidable), dropping
    // the §7.3.7 same-OrderHintBits judgment, so celu/doh-order-hint-bits-mismatch is SILENT
    // across every temporal unit. Pre-fix `frame_order_hint_bits` returned the stale active
    // header's bits (2) regardless of resolution, so TU3 saw bits {1, 2} and false-positived.
    let mut data = temporal_delimiter_obu(); // TU1: xlayer 0 only (bits 1)
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(celu_seq_header_obu(0, 0, 1)); // xlayer 0 active: seq 0, OrderHintBits 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // resolves seq 0
    data.extend(temporal_delimiter_obu()); // TU2: xlayer 1 only (bits 2)
    data.extend(celu_seq_header_obu(1, 1, 2)); // xlayer 1 active: seq 1, OrderHintBits 2
    data.extend(celu_output_clk_ref(1, 1, 2, 0)); // resolves seq 1
    data.extend(temporal_delimiter_obu()); // TU3: the test temporal unit
    // xlayer 0 resolves its (resent) active header -> contributes bits 1.
    data.extend(celu_seq_header_obu(0, 0, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0));
    // xlayer 1 frame references the ABSENT seq 5 (parsed against the stale seq 1, bits 2);
    // the referenced-header guard returns Unknown, so post-fix it contributes no bits.
    data.extend(celu_seq_header_obu(1, 1, 2));
    data.extend(celu_output_clk_ref(1, 5, 2, 0));
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-bits-mismatch"),
        "a frame referencing an absent header must contribute no OrderHintBits (not the \
         stale active header's bits), so the bits-mismatch rule must stay silent; \
         report was: {report}"
    );
}

#[test]
fn celu_doh_order_hint_bits_resolved_frames_with_differing_bits_still_fires() {
    // F4 (negative control): the fix must not over-suppress. Two RESOLVED frames in one
    // temporal unit under the DOH flag whose active headers declare different OrderHintBits
    // (1 vs 2) still fire celu/doh-order-hint-bits-mismatch — both frames resolve to their
    // active headers, so both contribute their (differing) bits.
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(celu_seq_header_obu(0, 0, 1)); // xlayer 0: OrderHintBits 1
    data.extend(celu_seq_header_obu(1, 1, 2)); // xlayer 1: OrderHintBits 2
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // resolves seq 0 -> bits 1
    data.extend(celu_output_clk_ref(1, 1, 2, 0)); // resolves seq 1 -> bits 2
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-bits-mismatch"),
        "two resolved frames with differing known OrderHintBits must still fire the \
         bits-mismatch rule; report was: {report}"
    );
}

// --- F1: OrderHintBits noted per frame UNIT, not per OBU (§ 7.3.7) ------------------

/// A non-first tile group (`is_first_tile_group == 0`) CLK on `xlayer` (mlayer 0):
/// the continuation of an already-open coded frame. The frame-core parser sees the
/// `0` first bit and returns `None`, so this OBU contributes no resolved facts and no
/// OrderHintBits — it is a [`FrameBoundary::ContinuesUnit`] of the open unit.
pub(in crate::validator::tests) fn celu_clk_non_first_tile(xlayer: u8) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(0); // is_first_tile_group == 0 -> non-first tile group (continuation)
    annex_b_obu_with_header(&layer_obu_header(4, 0, 0, xlayer), &fb.into_bytes())
}

#[test]
fn celu_doh_order_hint_bits_continuation_obu_does_not_poison_unit() {
    // F1: the per-frame OrderHintBits must be fed to the §7.3.7 accumulator per frame
    // UNIT, not per OBU. A non-first tile group (is_first_tile_group == 0) parses no
    // frame core, so it contributes None — but it is a CONTINUATION of an already-open
    // coded frame (FrameBoundary::ContinuesUnit), whose OrderHintBits came from its
    // opener. Pre-fix the validator noted the continuation's None for every frame-bearing
    // OBU, setting bits_undecidable and POISONING the whole temporal unit's §7.3.7
    // same-OrderHintBits judgment — over-suppression that hid a real bits mismatch.
    //
    // Two active headers are established in separate temporal units so no single TU
    // legitimately carries two OrderHintBits: TU1 activates seq 0 (OrderHintBits 1) for
    // xlayer 0; TU2 activates seq 1 (OrderHintBits 2) for xlayer 1. An MSDO with
    // multistream_doh_constraint_flag == 1 opens a persisting CMVS. TU3 (CMVS open) is the
    // test temporal unit: xlayer 0's coded frame is SPLIT into a first tile group (bits 1)
    // and a non-first tile group (None, the continuation); xlayer 1's frame contributes
    // bits 2. Post-fix the continuation is skipped, so the TU sees bits {1, 2} and
    // celu/doh-order-hint-bits-mismatch FIRES. Pre-fix the continuation's None poisoned the
    // judgment and the mismatch was silent.
    let mut data = temporal_delimiter_obu(); // TU1: xlayer 0 (bits 1)
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(celu_seq_header_obu(0, 0, 1)); // xlayer 0 active: seq 0, OrderHintBits 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // resolves seq 0
    data.extend(temporal_delimiter_obu()); // TU2: xlayer 1 (bits 2)
    data.extend(celu_seq_header_obu(1, 1, 2)); // xlayer 1 active: seq 1, OrderHintBits 2
    data.extend(celu_output_clk_ref(1, 1, 2, 0)); // resolves seq 1
    data.extend(temporal_delimiter_obu()); // TU3: the test temporal unit
    data.extend(celu_seq_header_obu(0, 0, 1));
    // xlayer 0 coded frame: first tile group (bits 1) + non-first tile group (None).
    data.extend(celu_output_clk_ref(0, 0, 1, 0));
    data.extend(celu_clk_non_first_tile(0));
    // xlayer 1 frame: contributes bits 2.
    data.extend(celu_seq_header_obu(1, 1, 2));
    data.extend(celu_output_clk_ref(1, 1, 2, 0));
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-bits-mismatch"),
        "a non-first tile group (a continuation) must not poison the temporal unit's \
         OrderHintBits judgment, so the real bits mismatch must fire; report was: {report}"
    );
}

#[test]
fn celu_doh_order_hint_bits_continuation_only_does_not_poison() {
    // F1 (negative control): a single coded frame split into first + non-first tile groups
    // at one xlayer with one OrderHintBits must stay silent — the continuation must not
    // be noted as a None contribution (which would set bits_undecidable but also, with a
    // single resolved opener, leave no mismatch to fire), and must not itself fire.
    let mut data = temporal_delimiter_obu();
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(celu_seq_header_obu(0, 0, 1)); // xlayer 0 active: OrderHintBits 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // first tile group
    data.extend(celu_clk_non_first_tile(0)); // non-first tile group (continuation)
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-bits-mismatch"),
        "a normal split frame (one OrderHintBits) must stay silent; report was: {report}"
    );
}

// --- Round-5 F3: a global frame-bearing OBU must not poison the DOH bits accumulator -

/// A frame-bearing OBU (a CLK tile group) carried at `obu_xlayer_id == GLOBAL_XLAYER_ID`
/// (31). This is invalid (a CLK may not use the global xlayer — see
/// `obu-header/global-xlayer-allowed-types`), and global OBUs are not part of any CELU
/// (§ 7.3.6); the frame core never resolves an active sequence header for xlayer 31, so it
/// carries no decidable OrderHintBits. Its first-tile-group prefix opens a (would-be)
/// frame unit, so the segmenter reports an `OpensNewUnit`-class boundary feeding the DOH
/// accumulator a `None` contribution at the wiring seam before the CELU tracker's
/// non-global filter (round-5 F3).
pub(in crate::validator::tests) fn celu_global_frame_obu() -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group -> opens a (would-be) frame unit
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(7); // seq_header_id_in_frame_header -> absent for xlayer 31 (never resolves)
    annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 31), &fb.into_bytes())
}

#[test]
fn celu_doh_global_frame_obu_does_not_poison_bits_accumulator() {
    // Round-5 F3: a frame-bearing OBU at obu_xlayer_id == GLOBAL_XLAYER_ID (31) must not
    // feed the § 7.3.7 OrderHintBits accumulator. Such an OBU is not part of any CELU
    // (§ 7.3.6, the CELU tracker filters globals in `observe`), but the wiring fed
    // `note_order_hint_bits` BEFORE that filter, so the global OBU contributed a `None`
    // (no sequence header is active for xlayer 31) that set `bits_undecidable` and
    // suppressed a real cross-CELU bits mismatch between the valid CELUs in the TU.
    //
    // Two active headers are established in separate temporal units so no single TU
    // legitimately carries two OrderHintBits: TU1 activates seq 0 (OrderHintBits 1) for
    // xlayer 0; TU2 activates seq 1 (OrderHintBits 2) for xlayer 1. An MSDO with
    // multistream_doh_constraint_flag == 1 opens a persisting CMVS. TU3 (CMVS open) is the
    // test temporal unit: xlayer 0 contributes bits 1, xlayer 1 contributes bits 2, plus a
    // global frame-bearing OBU. Post-fix the global OBU is skipped, so the TU sees bits
    // {1, 2} and celu/doh-order-hint-bits-mismatch FIRES. Pre-fix the global OBU's None
    // poisoned the judgment and the mismatch was silent.
    let mut data = temporal_delimiter_obu(); // TU1: xlayer 0 (bits 1)
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(celu_seq_header_obu(0, 0, 1)); // xlayer 0 active: seq 0, OrderHintBits 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // resolves seq 0
    data.extend(temporal_delimiter_obu()); // TU2: xlayer 1 (bits 2)
    data.extend(celu_seq_header_obu(1, 1, 2)); // xlayer 1 active: seq 1, OrderHintBits 2
    data.extend(celu_output_clk_ref(1, 1, 2, 0)); // resolves seq 1
    data.extend(temporal_delimiter_obu()); // TU3: the test temporal unit
    data.extend(celu_seq_header_obu(0, 0, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // xlayer 0 -> bits 1
    data.extend(celu_global_frame_obu()); // global frame-bearing OBU -> must NOT poison
    data.extend(celu_seq_header_obu(1, 1, 2));
    data.extend(celu_output_clk_ref(1, 1, 2, 0)); // xlayer 1 -> bits 2
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-bits-mismatch"),
        "a global frame-bearing OBU must not poison the § 7.3.7 OrderHintBits accumulator, \
         so the real bits mismatch between the valid CELUs must fire; report was: {report}"
    );
    // The global frame-bearing OBU keeps its own header diagnostic (orthogonal).
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-header/global-xlayer-allowed-types"),
        "the global frame-bearing OBU must still trip its own obu-header diagnostic; \
         report was: {report}"
    );
}

// --- Round-5 F1: DOH flag sampled against the GOVERNING (pre-completion) CMVS window -

#[test]
fn celu_doh_flag_via_lcr_survives_cmvs_close_at_boundary_tu() {
    // Round-5 F1: at a global-temporal-delimiter boundary the DOH constraint flag for the
    // just-completed temporal unit must be sampled against the CMVS window that CONTAINS
    // that temporal unit — captured BEFORE the CMVS tracker applies the unit's § 7.3.2
    // begin/end conditions and clears the live window.
    //
    // § 7.3.2: a coded multistream video sequence "ends at" the temporal unit that begins
    // a new coded video sequence (a CLK) but has no OBU_MSDO and no activated global LCR
    // (end condition 2). That boundary temporal unit is the LAST temporal unit of the
    // ENDING CMVS — it is contained in the old CMVS, not a new one (no begin condition
    // fired). § 7.3.7's DOH OrderHint / OrderHintBits constraints apply "for each temporal
    // unit in the coded multistream video sequence", so the flag for this last unit comes
    // from the OLD CMVS's activated global LCR. Sampling it against the live window AFTER
    // the tracker Closed that window resolves `activated_global_lcr()` to None and skips the
    // § 7.3.7 checks (a false negative).
    //
    // TU0 opens an Inside CMVS via begin condition 1 (a CLK xlayer 0 + an MSDO), with a
    // global LCR id 1 whose lcr_doh_constraint_flag == 1 activated by both layers' headers
    // (seq_lcr_id == 1). The MSDO's multistream_doh_constraint_flag is 0, so the DOH flag
    // can come ONLY from the activated global LCR (isolating the LCR-window path). TU1 is
    // the test temporal unit: two output CLKs (xlayer 0 OrderHint 0, xlayer 1 OrderHint 1,
    // equal known OrderHintBits 1) carry a cross-CELU OrderHint mismatch, and TU1 carries no
    // MSDO — a CLK-with-no-MSDO temporal unit, so it ENDS the CMVS (the boundary TU). The
    // activated global LCR (lcr_doh_constraint_flag 1, observed in TU0) governs TU1, so the
    // § 7.3.7 cross-CELU OrderHint check must FIRE. Pre-fix the live window was Closed
    // before the flag was sampled, so the flag read false and the mismatch was silent.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, true); // doh flag 1, xlayers 0,1
    let mut data = temporal_delimiter_obu(); // TU0: opens an Inside CMVS (begin condition 1)
    data.extend(global);
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)])); // MSDO doh 0
    // Per-layer coded extended layer units in ascending obu_xlayer_id order (§ 7.3.7):
    // seq0 + CLK0, then seq1 + CLK1. Both CLKs frame-confirm their headers (seq_lcr_id 1),
    // activating the global LCR for the CMVS.
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // seq 0 xlayer 0, seq_lcr_id 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // CLK xlayer 0 -> activates seq 0 / LCR 1
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // seq 1 xlayer 1, seq_lcr_id 1
    data.extend(celu_output_clk_ref(1, 1, 1, 0)); // CLK xlayer 1 -> activates seq 1 / LCR 1
    data.extend(temporal_delimiter_obu()); // TU1: the boundary TU that ends the CMVS
    // No MSDO in TU1 -> a CLK-with-no-MSDO temporal unit ends the CMVS (end condition 2).
    // Re-send each activatable header before its CELU's frame so TU1's frames resolve.
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // CELU xlayer 0, OrderHint 0
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
    data.extend(celu_output_clk_ref(1, 1, 1, 1)); // CELU xlayer 1, OrderHint 1 -> mismatch
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-mismatch"),
        "the activated global LCR (lcr_doh_constraint_flag 1) governs the CMVS-ending \
         boundary temporal unit, so the § 7.3.7 cross-CELU OrderHint check must fire even \
         though that unit closes the live CMVS window; report was: {report}"
    );
}

#[test]
fn celu_doh_flag_via_lcr_off_at_boundary_tu_stays_silent() {
    // Round-5 F1 (precision): the governing-window capture must NOT blanket-enable the DOH
    // checks. The identical stream as `…_survives_cmvs_close_at_boundary_tu`, but the
    // activated global LCR's lcr_doh_constraint_flag == 0 (and the MSDO's flag is 0). The
    // boundary temporal unit's governing CMVS therefore declares NO DOH constraint, so the
    // cross-CELU OrderHint disagreement is conforming and celu/doh-order-hint-mismatch must
    // stay SILENT — the fix samples the flag against the governing window, it does not
    // unconditionally treat the boundary unit as flagged.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh flag 0
    let mut data = temporal_delimiter_obu();
    data.extend(global);
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)])); // MSDO doh 0
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0));
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
    data.extend(celu_output_clk_ref(1, 1, 1, 0));
    data.extend(temporal_delimiter_obu()); // boundary TU ends the CMVS (no MSDO)
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // OrderHint 0
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
    data.extend(celu_output_clk_ref(1, 1, 1, 1)); // OrderHint 1 (would mismatch if flagged)
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-mismatch"),
        "with lcr_doh_constraint_flag == 0 the boundary unit's governing CMVS declares no \
         DOH constraint, so the cross-CELU OrderHint check must stay silent; report was: \
         {report}"
    );
}

#[test]
fn celu_doh_flag_via_lcr_survives_eof_close_at_boundary_tu() {
    // Round-6 F1: the END-OF-STREAM analogue of
    // `…_survives_cmvs_close_at_boundary_tu`. Identical stream EXCEPT the boundary
    // temporal unit (the CLK-with-no-MSDO TU that ENDS the CMVS) is the FINAL temporal
    // unit — there is no trailing global temporal delimiter, so the validator's
    // end-of-stream `finish` path (not the internal temporal-delimiter path) completes
    // it. § 7.3.2 end condition 3 ("the end of the bitstream") ends the CMVS, so this
    // final unit is the LAST temporal unit of the ENDING CMVS and is governed by that
    // CMVS's activated global LCR (lcr_doh_constraint_flag 1). The § 7.3.7 cross-CELU
    // OrderHint check must therefore FIRE. Pre-fix the EOF path sampled the DOH flag
    // against the LIVE window AFTER `cmvs.complete_temporal_unit` cleared it (end
    // condition 3 closes the live window), so the flag read false and the mismatch was
    // silently suppressed — the internal-boundary governing-window capture (round-5 F1)
    // was never mirrored to the EOF path.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, true); // doh flag 1, xlayers 0,1
    let mut data = temporal_delimiter_obu(); // TU0: opens an Inside CMVS (begin condition 1)
    data.extend(global);
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)])); // MSDO doh 0
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // seq 0 xlayer 0, seq_lcr_id 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // CLK xlayer 0 -> activates seq 0 / LCR 1
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // seq 1 xlayer 1, seq_lcr_id 1
    data.extend(celu_output_clk_ref(1, 1, 1, 0)); // CLK xlayer 1 -> activates seq 1 / LCR 1
    data.extend(temporal_delimiter_obu()); // TU1: the FINAL TU that ends the CMVS at EOF
    // No MSDO in TU1 -> a CLK-with-no-MSDO temporal unit ends the CMVS (end condition 2),
    // and the end of the bitstream ends it (end condition 3) with NO trailing delimiter.
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // CELU xlayer 0, OrderHint 0
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
    data.extend(celu_output_clk_ref(1, 1, 1, 1)); // CELU xlayer 1, OrderHint 1 -> mismatch
    // NO trailing temporal delimiter: the stream ENDS on the boundary TU.
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-mismatch"),
        "the activated global LCR (lcr_doh_constraint_flag 1) governs the CMVS-ending \
         FINAL temporal unit, so the § 7.3.7 cross-CELU OrderHint check must fire even \
         though the end of the bitstream closes the live CMVS window; report was: {report}"
    );
}

#[test]
fn celu_doh_flag_via_lcr_off_at_eof_boundary_tu_stays_silent() {
    // Round-6 F1 (precision): the EOF governing-window capture must NOT blanket-enable
    // the DOH checks. Identical to `…_survives_eof_close_at_boundary_tu` but the
    // activated global LCR's lcr_doh_constraint_flag == 0 (MSDO flag also 0), so the
    // final temporal unit's governing CMVS declares NO DOH constraint — the cross-CELU
    // OrderHint disagreement is conforming and celu/doh-order-hint-mismatch must stay
    // SILENT.
    let global = global_lcr_obu_agreement(1, 0b11, None, None, false); // doh flag 0
    let mut data = temporal_delimiter_obu();
    data.extend(global);
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)])); // MSDO doh 0
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0));
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
    data.extend(celu_output_clk_ref(1, 1, 1, 0));
    data.extend(temporal_delimiter_obu()); // FINAL TU ends the CMVS (no MSDO), no trailing TD
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // OrderHint 0
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
    data.extend(celu_output_clk_ref(1, 1, 1, 1)); // OrderHint 1 (would mismatch if flagged)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/doh-order-hint-mismatch"),
        "with lcr_doh_constraint_flag == 0 the final boundary unit's governing CMVS \
         declares no DOH constraint, so the cross-CELU OrderHint check must stay silent; \
         report was: {report}"
    );
}

// --- F2: MFH-backed frames are decided when the in-band MFH resolves (§ 7.3.6) -----

/// A CLK frame-bearing OBU on `xlayer` (mlayer 0) whose frame header references the
/// multi-frame header `cur_mfh_id` (> 0) and reaches the intra output flags. With
/// `monotonic_output_order_flag == 1` and `immediate_output == false` the frame settles
/// a DECIDED non-output class. The active sequence header (resolved via the MFH) has
/// `OrderHintBits == order_hint_bits`.
pub(in crate::validator::tests) fn celu_clk_mfh_ref(
    xlayer: u8,
    cur_mfh_id: u32,
    order_hint_bits: u32,
    order_hint: u32,
    immediate_output: bool,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(cur_mfh_id); // cur_mfh_id > 0 -> no seq_header_id_in_frame_header
    fb.bit(u8::from(immediate_output)); // immediate_output_frame
    fb.bit(0); // frame_size_override_flag
    fb.f(order_hint, order_hint_bits); // order_hint f(OrderHintBits)
    fb.bit(0); // allow_screen_content_tools
    fb.bit(0); // allow_intrabc
    fb.bit(0); // disable_cdf_update
    intra_structure_tail(&mut fb, 0);
    // The cur_mfh_id > 0 prefix (`uvlc(cur_mfh_id)`) is one bit longer than the
    // direct cur_mfh_id == 0 prefix, so the now-fully-parsed § 5.18.2 intra tail and
    // the loop-filter cluster (deblocking_filter_params() reads 2 apply bits with
    // GDF/CDEF disabled; lr_params()/ccso_params() read nothing with restoration/CCSO
    // disabled) can need more bits than byte-padding alone supplies; an extra padding
    // byte gives the core parser room to reach its StoppedBeforeReadTxMode stop
    // (trailing bits past the stop are ignored).
    fb.f(0, 8);
    annex_b_obu_with_header(&layer_obu_header(4, 0, 0, xlayer), &fb.into_bytes())
}

#[test]
fn celu_mfh_backed_frame_is_decided_not_unknown() {
    // F2: an in-band multi-frame header whose mfh_seq_header_id equals the active header,
    // plus a cur_mfh_id == 1 frame whose output flags ARE parseable, must let the §7.3.6
    // presence checks DECIDE rather than route to Unknown. The §5.18.2 core parser reaches
    // the intra output flags using the active (MFH-resolved) sequence header alone — the
    // cur_mfh_id > 0 stop is later (segmentation), past the output flags — so the output
    // class is decidable. Here the cur_mfh_id == 1 CLK is a DECIDED non-output frame
    // (immediate_output == 0, monotonic order), so its CELU has a decided non-output unit
    // and no output unit -> celu/missing-output-frame-unit fires.
    //
    // Pre-fix frame_core_against_referenced_header required
    // referenced_sequence_header_id == Some(seq_id), which is None for every cur_mfh_id > 0
    // frame, so the frame routed to Unknown and the presence check was silently dropped.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1))); // seq 0
    data.extend(multi_frame_header_obu(0)); // in-band MFH: mfhId 1 -> mfh_seq_header_id 0
    // cur_mfh_id 1 -> resolves MFH 1 -> seq 0 (== active header); decided non-output.
    data.extend(celu_clk_mfh_ref(0, 1, 1, 0, false));
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "celu/missing-output-frame-unit"),
        "an in-band MFH-backed frame whose output flags are parseable must DECIDE the \
         §7.3.6 presence check rather than route to Unknown; report was: {report}"
    );
}

#[test]
fn celu_mfh_referencing_different_seq_than_active_is_unknown() {
    // F2 (negative control): the in-band MFH's mfh_seq_header_id resolves to a sequence
    // header that is NOT the active header the frame parses against, so the resolution check
    // fails and the frame stays Unknown. TU1 activates seq 0 for xlayer 0 via a direct
    // cur_mfh_id == 0 CLK. TU2 carries an in-band MFH whose mfh_seq_header_id is 1 — but
    // seq 1 is never sent in-band, so the frame's MFH reference does NOT activate it and the
    // stale active header stays seq 0. The cur_mfh_id == 1 frame would, parsed against the
    // stale seq 0, settle a decided non-output class; but because the MFH's
    // mfh_seq_header_id (1) != the parsed-against active id (0) the guard returns Unknown,
    // so celu/missing-output-frame-unit must NOT fire from a misparse.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1))); // seq 0
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // TU1: activates seq 0 for xlayer 0
    data.extend(temporal_delimiter_obu()); // TU2
    data.extend(multi_frame_header_obu(1)); // in-band MFH: mfhId 1 -> mfh_seq_header_id 1 (seq 1 absent)
    // cur_mfh_id 1 -> resolves MFH 1 -> seq 1 (unavailable), so the frame stays on the stale
    // active seq 0; mfh_seq_header_id (1) != parsed-against id (0) -> Unknown.
    data.extend(celu_clk_mfh_ref(0, 1, 1, 0, false));
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/missing-output-frame-unit"),
        "an MFH resolving to a header other than the active one must stay Unknown; \
         report was: {report}"
    );
}

#[test]
fn celu_mfh_out_of_range_or_absent_is_unknown() {
    // F2 (negative control): a cur_mfh_id whose MFH record is absent in-band (no MFH OBU)
    // stays Unknown — there is no in-band record to resolve, so the frame's output class is
    // undecidable and celu/missing-output-frame-unit must not fire. (The unavailable-MFH
    // diagnostic is orthogonal.)
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1))); // seq 0
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // activate seq 0 for xlayer 0
    // cur_mfh_id 2: no in-band MFH record -> unavailable, undecidable -> Unknown.
    data.extend(celu_clk_mfh_ref(0, 2, 1, 0, false));
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "celu/missing-output-frame-unit"),
        "a frame whose cur_mfh_id has no in-band MFH record must stay Unknown; \
         report was: {report}"
    );
}

// --- F3: mlayer ordering covers ContinuesUnit / Ambiguous frame OBUs (§ 7.3.6) -----

/// A CLK on `(mlayer, xlayer 0)` referencing the directly-resolved `seq_header_id`,
/// is_first_tile_group == 1, with no parseable body (activation-prefix only). Used as a
/// frame-unit opener at a chosen embedded layer for the ascending-mlayer ordering tests.
pub(in crate::validator::tests) fn celu_clk_layer_prefix(
    mlayer: u8,
    seq_header_id: u32,
) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(seq_header_id); // seq_header_id_in_frame_header
    annex_b_obu_with_header(&layer_obu_header(4, 0, mlayer, 0), &fb.into_bytes())
}

/// A non-first tile group (is_first_tile_group == 0) CLK on `(mlayer, xlayer 0)`: a
/// continuation of the open coded frame in that embedded layer.
pub(in crate::validator::tests) fn celu_clk_layer_non_first_tile(mlayer: u8) -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(0); // is_first_tile_group == 0 -> continuation
    annex_b_obu_with_header(&layer_obu_header(4, 0, mlayer, 0), &fb.into_bytes())
}

#[test]
fn celu_in_unit_order_continuation_at_lower_mlayer_after_higher_fires() {
    // F3: a continuation OBU (is_first_tile_group == 0, FrameBoundary::ContinuesUnit) still
    // belongs to its embedded layer's frame unit and is boundary-independent evidence for
    // the §7.3.6 ascending-obu_mlayer_id check. Interleave: mlayer 0 first tile, mlayer 1
    // frame, mlayer 0 NON-first tile. The trailing mlayer-0 continuation arrives after a
    // mlayer-1 frame unit began, so it is out of ascending order -> celu/in-unit-order.
    // Pre-fix observe_frame's ContinuesUnit early return skipped note_embedded_layer_ordering,
    // so the continuation's mlayer was never compared and the violation was silent.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1))); // seq 0, mlayer 0/1 ok
    data.extend(celu_clk_layer_prefix(0, 0)); // mlayer 0: first tile group (opens unit)
    data.extend(celu_clk_layer_prefix(1, 0)); // mlayer 1: frame unit begins
    data.extend(celu_clk_layer_non_first_tile(0)); // mlayer 0: continuation after mlayer 1
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == "celu/in-unit-order"),
        "a continuation OBU at a lower mlayer after a higher embedded layer began must \
         fire the ascending-mlayer order check; report was: {report}"
    );
}

#[test]
fn celu_in_unit_order_normal_split_frame_stays_silent() {
    // F3 (negative control): a normal split frame at one embedded layer (first + non-first
    // tile group, no interleaving) must stay silent — the continuation shares its opener's
    // mlayer, so it never lowers the high-water mark.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(celu_clk_layer_prefix(0, 0)); // mlayer 0: first tile group
    data.extend(celu_clk_layer_non_first_tile(0)); // mlayer 0: non-first tile group
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report.errors().any(|d| d.rule_id == "celu/in-unit-order"),
        "a normal split frame at one embedded layer must stay silent; report was: {report}"
    );
}

#[test]
fn celu_in_unit_order_ambiguous_obu_at_lower_mlayer_after_higher_fires() {
    // F3: an Ambiguous frame OBU belongs to SOME layer-m unit either way, so it is also
    // boundary-independent evidence for the ascending-mlayer check. A same-type, no-in-band-
    // delimiter TIP OBU following an open same-type TIP coded frame is Ambiguous (the
    // segmenter cannot decide whether it continues or opens a new unit). Sequence: mlayer 0
    // TIP (opens), mlayer 1 frame (higher layer begins), mlayer 0 same-type TIP (Ambiguous)
    // -> the ambiguous OBU is at a lower mlayer after a higher one began -> celu/in-unit-order.
    // Pre-fix observe_frame's Ambiguous early return skipped note_embedded_layer_ordering.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    // OBU_REGULAR_TIP is type 14; it carries no is_first_tile_group delimiter.
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(14, 0, 0, 0),
        &[0x80],
    )); // mlayer 0 TIP (opens)
    data.extend(celu_clk_layer_prefix(1, 0)); // mlayer 1: frame unit begins
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(14, 0, 0, 0),
        &[0x80],
    )); // mlayer 0 same-type TIP (ambiguous)
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report.errors().any(|d| d.rule_id == "celu/in-unit-order"),
        "an ambiguous OBU at a lower mlayer after a higher embedded layer began must fire \
         the ascending-mlayer order check; report was: {report}"
    );
}
