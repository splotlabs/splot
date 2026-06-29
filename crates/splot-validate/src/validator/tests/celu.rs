// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

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

#[test]
fn celu_frame_referencing_absent_seq_header_is_unknown_not_misparsed() {
    let mut data = temporal_delimiter_obu();
    data.extend(celu_seq_header_obu(0, 0, 1)); // seq 0, OrderHintBits 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // TU1: activates seq 0 for xlayer 0
    data.extend(temporal_delimiter_obu()); // TU2
    data.extend(celu_clk_ref(0, 1, 1, 0, false));
    data.extend(temporal_delimiter_obu());
    let report = Validator::new(false).validate_bytes(&data);
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

#[test]
fn celu_doh_order_hint_bits_absent_ref_frame_contributes_no_bits() {
    let mut data = temporal_delimiter_obu(); // TU1: xlayer 0 only (bits 1)
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(celu_seq_header_obu(0, 0, 1)); // xlayer 0 active: seq 0, OrderHintBits 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // resolves seq 0
    data.extend(temporal_delimiter_obu()); // TU2: xlayer 1 only (bits 2)
    data.extend(celu_seq_header_obu(1, 1, 2)); // xlayer 1 active: seq 1, OrderHintBits 2
    data.extend(celu_output_clk_ref(1, 1, 2, 0)); // resolves seq 1
    data.extend(temporal_delimiter_obu()); // TU3: the test temporal unit
    data.extend(celu_seq_header_obu(0, 0, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0));
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
    let mut data = temporal_delimiter_obu(); // TU1: xlayer 0 (bits 1)
    data.extend(msdo_obu_configured(0, true, &[(0, 0, 0, 0), (1, 0, 0, 0)]));
    data.extend(celu_seq_header_obu(0, 0, 1)); // xlayer 0 active: seq 0, OrderHintBits 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // resolves seq 0
    data.extend(temporal_delimiter_obu()); // TU2: xlayer 1 (bits 2)
    data.extend(celu_seq_header_obu(1, 1, 2)); // xlayer 1 active: seq 1, OrderHintBits 2
    data.extend(celu_output_clk_ref(1, 1, 2, 0)); // resolves seq 1
    data.extend(temporal_delimiter_obu()); // TU3: the test temporal unit
    data.extend(celu_seq_header_obu(0, 0, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0));
    data.extend(celu_clk_non_first_tile(0));
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

/// A frame-bearing OBU (a CLK tile group) carried at `obu_xlayer_id == GLOBAL_XLAYER_ID`
/// (31). This is invalid (a CLK may not use the global xlayer — see
/// `obu-header/global-xlayer-allowed-types`), and global OBUs are not part of any CELU
/// (§ 7.3.6); the frame core never resolves an active sequence header for xlayer 31, so it
/// carries no decidable OrderHintBits. Its first-tile-group prefix opens a (would-be)
/// frame unit, so the segmenter reports an `OpensNewUnit`-class boundary feeding the DOH
/// accumulator a `None` contribution at the wiring seam before the CELU tracker's
/// non-global filter.
pub(in crate::validator::tests) fn celu_global_frame_obu() -> Vec<u8> {
    let mut fb = Bits::default();
    fb.bit(1); // is_first_tile_group -> opens a (would-be) frame unit
    fb.uvlc(0); // cur_mfh_id == 0
    fb.uvlc(7); // seq_header_id_in_frame_header -> absent for xlayer 31 (never resolves)
    annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 31), &fb.into_bytes())
}

#[test]
fn celu_doh_global_frame_obu_does_not_poison_bits_accumulator() {
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
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-header/global-xlayer-allowed-types"),
        "the global frame-bearing OBU must still trip its own obu-header diagnostic; \
         report was: {report}"
    );
}

#[test]
fn celu_doh_flag_via_lcr_survives_cmvs_close_at_boundary_tu() {
    let global = global_lcr_obu_agreement(1, 0b11, None, None, true); // doh flag 1, xlayers 0,1
    let mut data = temporal_delimiter_obu(); // TU0: opens an Inside CMVS (begin condition 1)
    data.extend(global);
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)])); // MSDO doh 0
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // seq 0 xlayer 0, seq_lcr_id 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // CLK xlayer 0 -> activates seq 0 / LCR 1
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // seq 1 xlayer 1, seq_lcr_id 1
    data.extend(celu_output_clk_ref(1, 1, 1, 0)); // CLK xlayer 1 -> activates seq 1 / LCR 1
    data.extend(temporal_delimiter_obu()); // TU1: the boundary TU that ends the CMVS
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
    let global = global_lcr_obu_agreement(1, 0b11, None, None, true); // doh flag 1, xlayers 0,1
    let mut data = temporal_delimiter_obu(); // TU0: opens an Inside CMVS (begin condition 1)
    data.extend(global);
    data.extend(msdo_obu_configured(0, false, &[(0, 0, 0, 0), (1, 0, 0, 0)])); // MSDO doh 0
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1)); // seq 0 xlayer 0, seq_lcr_id 1
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // CLK xlayer 0 -> activates seq 0 / LCR 1
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1)); // seq 1 xlayer 1, seq_lcr_id 1
    data.extend(celu_output_clk_ref(1, 1, 1, 0)); // CLK xlayer 1 -> activates seq 1 / LCR 1
    data.extend(temporal_delimiter_obu()); // TU1: the FINAL TU that ends the CMVS at EOF
    data.extend(seq_header_obu_lcr_ref(0, 0, 0, true, 1));
    data.extend(celu_output_clk_ref(0, 0, 1, 0)); // CELU xlayer 0, OrderHint 0
    data.extend(seq_header_obu_lcr_ref(1, 1, 0, true, 1));
    data.extend(celu_output_clk_ref(1, 1, 1, 1)); // CELU xlayer 1, OrderHint 1 -> mismatch
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
    fb.f(0, 8);
    annex_b_obu_with_header(&layer_obu_header(4, 0, 0, xlayer), &fb.into_bytes())
}

#[test]
fn celu_mfh_backed_frame_is_decided_not_unknown() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1))); // seq 0
    data.extend(multi_frame_header_obu(0)); // in-band MFH: mfhId 1 -> mfh_seq_header_id 0
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
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1))); // seq 0
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // TU1: activates seq 0 for xlayer 0
    data.extend(temporal_delimiter_obu()); // TU2
    data.extend(multi_frame_header_obu(1)); // in-band MFH: mfhId 1 -> mfh_seq_header_id 1 (seq 1 absent)
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
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1))); // seq 0
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // activate seq 0 for xlayer 0
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
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
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
