// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

// --- § 6.16.5 / § 6.16.6 first-coded-picture half ---

#[test]
fn metadata_hdr_cll_first_coded_picture_late_is_flagged() {
    // AV2 § 6.16.5: HDR CLL metadata associated with an embedded layer shall be
    // indicated at the first coded picture of that layer in the CVS. A
    // LAYER_CURRENT-targeted HDR CLL unit (obu_xlayer_id 0 / obu_mlayer_id 0)
    // first established AFTER that layer's first coded picture (a CLK) fires.
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // first coded picture (xlayer 0)
    data.extend(hdr_cll_unit_layer_current(0, 1000, 200)); // late establishment
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-cll-first-coded-picture"),
        "report was: {report}"
    );
}

#[test]
fn metadata_hdr_mdcv_first_coded_picture_late_is_flagged() {
    // AV2 § 6.16.6: same rule for HDR MDCV. A LAYER_CURRENT MDCV unit first
    // established after the layer's first coded picture (a CLK) fires.
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // first coded picture
    // metadata_short LAYER_CURRENT (0x20), metadata_type 2 (HDR_MDCV).
    let payload = metadata_short_payload(0x20, 2, &hdr_mdcv_unit(60));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 0),
        &payload,
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-mdcv-first-coded-picture"),
        "report was: {report}"
    );
}

#[test]
fn metadata_hdr_cll_at_first_coded_picture_is_conformant() {
    // The same HDR CLL unit indicated BEFORE the layer's first coded picture
    // (in the coded frame unit's pre-frame region) is conforming.
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(hdr_cll_unit_layer_current(0, 1000, 200)); // before the first picture
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-cll-first-coded-picture"),
        "report was: {report}"
    );
}

#[test]
fn metadata_hdr_cll_first_picture_in_new_cvs_is_conformant() {
    // AV2 § 6.16.5 / § 7.3.6: the "first coded picture of that embedded layer in
    // the coded video sequence" state is per CVS. A CLK starts a new CVS for its
    // extended layer at its temporal unit, but the new CVS's pre-frame HDR CLL is
    // processed BEFORE the CLK in stream order — when the previous CVS's
    // first-picture state is still present. The first-picture entry from the prior
    // CVS carries an earlier temporal-unit index, so the finding defers to the
    // temporal-unit flush, where the new CVS's CLK drops it. The new CVS's HDR CLL
    // (at its own first coded picture) must be silent. Pre-fix the stale prior-CVS
    // entry fired metadata/hdr-cll-first-coded-picture on a valid stream.
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // CVS 1 first picture (xlayer 0)
    data.extend(temporal_delimiter_obu()); // temporal-unit boundary (no clear here)
    data.extend(hdr_cll_unit_layer_current(0, 1000, 200)); // CVS 2 pre-frame HDR CLL
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // CVS 2 first picture (new CVS)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-cll-first-coded-picture"),
        "a new CVS's first-picture HDR CLL must not fire against stale prior-CVS \
         first-picture state; report was: {report}"
    );
}

#[test]
fn metadata_hdr_cll_suffix_in_first_coded_frame_unit_is_conformant() {
    // AV2 § 7.3.3 / § 6.16.5: the suffix-metadata tail is placed AFTER the coded
    // frame but still INSIDE the same coded frame unit. So a suffix HDR CLL that
    // follows the first coded picture's OBUs but is in that picture's own coded
    // frame unit is "indicated at the first coded picture" — it is NOT late. The
    // lateness predicate keys on coded-frame-UNIT boundaries, not first-frame-OBU
    // order. Pre-fix the suffix HDR CLL (after the CLK in stream order) fired
    // metadata/hdr-cll-first-coded-picture on a conforming stream.
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // first coded picture (xlayer 0)
    data.extend(hdr_cll_unit_layer_current_suffix(0, 1000, 200)); // suffix tail, same unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-cll-first-coded-picture"),
        "a suffix HDR CLL inside the first coded picture's own coded frame unit must \
         not be late; report was: {report}"
    );
}

#[test]
fn metadata_hdr_cll_suffix_in_second_unit_is_flagged() {
    // Control for the suffix-tail grace: a suffix HDR CLL in the SECOND coded frame
    // unit (after the first unit completed) is genuinely after the first coded
    // picture's unit, so it is still late. A first CLK (unit 1) completes when the
    // second CLK (unit 2) begins; the suffix HDR CLL then sits in unit 2's tail,
    // past the first picture's unit, so metadata/hdr-cll-first-coded-picture fires.
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // unit 1: first coded picture
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // unit 2: starts a new unit
    data.extend(hdr_cll_unit_layer_current_suffix(0, 1000, 200)); // unit 2 suffix tail: late
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-cll-first-coded-picture"),
        "a suffix HDR CLL in a later coded frame unit is still late; report was: {report}"
    );
}

#[test]
fn metadata_hdr_cll_suffix_after_later_temporal_layer_unit_is_flagged() {
    // AV2 § 6.16.5 / § 7.3.8.10: the suffix-tail grace counts COMPLETED coded frame
    // units per embedded layer across temporal layers (mirror line 880), not per
    // (xlayer, mlayer, tlayer) triple. The embedded layer (xlayer 0, mlayer 0) has
    // its first coded frame unit at obu_tlayer_id 0 (a CLK) and a SECOND coded frame
    // unit at obu_tlayer_id 1 (another CLK). A suffix HDR CLL after that later-tlayer
    // frame is in the layer's SECOND unit, past the first coded picture, so it is
    // late and metadata/hdr-cll-first-coded-picture must fire. Pre-fix the
    // completed-units count was only incremented when a new unit started within the
    // SAME triple state, so the cross-tlayer second unit left the count at 0, the
    // grace mis-fired, and the diagnostic was skipped.
    let mut data = td_and_seq_header(0, 1, 0); // max_tlayer_id == 1
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // tlayer 0: first coded picture
    // A CLK (obu_type 4) at obu_tlayer_id 1, same embedded layer (xlayer 0, mlayer 0).
    data.extend(frame_obu_direct_seq_ref_layer(4, 1, 0, 0, 0)); // tlayer 1: 2nd coded frame unit
    data.extend(hdr_cll_unit_layer_current_suffix(0, 1000, 200)); // suffix tail in 2nd unit: late
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-cll-first-coded-picture"),
        "a suffix HDR CLL after a later-temporal-layer coded frame unit of the same \
         embedded layer is past the first coded picture and must fire; report was: {report}"
    );
}

/// A local `OBU_METADATA_GROUP` at extended layer 0 / embedded layer 0 carrying a
/// single `LAYER_VALUES` HDR CLL unit (metadata_type 1) whose `muh_mlayer_map`
/// targets the embedded layers selected by `mlayer_map`. No temporal-delimiter
/// prefix (chainable). A non-global metadata OBU needs an active sequence header
/// for its extended layer.
pub(in crate::validator::tests) fn local_hdr_cll_group_layer_values_obu(
    mlayer_map: u8,
    max_cll: u32,
    max_fall: u32,
) -> Vec<u8> {
    let mut unit = Bits::default();
    unit.f(max_cll, 16);
    unit.f(max_fall, 16);
    let unit = unit.into_bytes();
    let payload_size = unit.len() as u8;
    // Local form: muh_header_size = leb(1) + fixed 2 + one muh_mlayer_map byte = 4,
    // so the muh_header_size+cancel byte is 4 << 1 = 0x08.
    let mut payload = vec![
        0x00, // group header (is_suffix=0, necessity=0, application_id=0)
        0x00, // metadata_unit_cnt_minus_1 = 0
        0x01, // metadata_type = 1 (METADATA_TYPE_HDR_CLL)
        0x08, // muh_header_size = 4, cancel = 0
        payload_size,
        0x60,
        0x00,       // layer_idc=LAYER_VALUES(3), persistence=0, priority=0, reserved=0
        mlayer_map, // muh_mlayer_map for this extended layer
    ];
    payload.extend_from_slice(&unit);
    payload.push(0x80); // OBU trailing byte
    annex_b_obu_with_header(&layer_obu_header(9, 0, 0, 0), &payload)
}

#[test]
fn metadata_hdr_cll_layer_values_late_for_one_targeted_layer_is_flagged() {
    // AV2 § 6.16.5: the first-coded-picture rule applies INDEPENDENTLY per
    // associated embedded layer. A LAYER_VALUES HDR CLL targeting embedded layers
    // (0, 0) and (0, 1) is late for layer (0, 0) — its first coded picture (a CLK)
    // already passed — even though layer (0, 1) has no coded picture yet. The
    // finding must fire for the late layer. Pre-fix the `all targeted layers seen`
    // gate suppressed it because (0, 1) was unseen.
    let mut data = td_and_seq_header(0, 0, 1); // max_mlayer_id = 1 allows mlayers 0 and 1
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // layer (0,0) first picture
    // Local group HDR CLL targeting mlayers 0 and 1 (map 0b011), late for (0,0).
    data.extend(local_hdr_cll_group_layer_values_obu(0b0000_0011, 1000, 200));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-cll-first-coded-picture"),
        "a LAYER_VALUES unit late for one targeted layer must fire; report was: {report}"
    );
}

/// A single-OBU CLK frame at xlayer 0 / `mlayer` referencing sequence header 0
/// directly. Used as the first coded picture of embedded layer `(0, mlayer)`.
pub(in crate::validator::tests) fn clk_frame_at_mlayer(mlayer: u8) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(1); // is_first_tile_group
    bits.uvlc(0); // cur_mfh_id == 0 -> direct sequence-header reference
    bits.uvlc(0); // seq_header_id_in_frame_header == 0
    // OBU_CLOSED_LOOP_KEY (type 4) with an extension header at (xlayer 0, mlayer).
    annex_b_obu_with_header(&layer_obu_header(4, 0, mlayer, 0), &bits.into_bytes())
}

#[test]
fn metadata_hdr_cll_layer_values_new_layer_late_past_established_layer_is_flagged() {
    // AV2 § 6.16.5: the first-coded-picture rule binds INDEPENDENTLY per associated
    // embedded layer (finding 4). A LAYER_VALUES HDR CLL targeting {layer (0,0) +
    // NEW layer (0,1)} arrives after BOTH layers' first coded pictures, but layer
    // (0,0)'s content was ALREADY established by an earlier baseline (targeting only
    // (0,0), indicated before its first picture). Layer (0,1) is freshly established
    // and late, so the finding must fire naming layer (0,1). Pre-fix the whole-unit
    // `any(intersects)` gate saw the layer-(0,0) baseline overlap and suppressed the
    // ENTIRE first-coded-picture check, missing the late new layer.
    let mut data = td_and_seq_header(0, 0, 1); // mlayers 0 and 1 valid
    // Baseline for layer (0,0) only, BEFORE its first picture -> timely, silent.
    data.extend(local_hdr_cll_group_layer_values_obu(0b0000_0001, 1000, 200));
    data.extend(clk_frame_at_mlayer(0)); // layer (0,0) first picture
    data.extend(clk_frame_at_mlayer(1)); // layer (0,1) first picture
    // Late unit targeting {0, 1}; identical layer-(0,0) content so no repeat-differs.
    data.extend(local_hdr_cll_group_layer_values_obu(0b0000_0011, 1000, 200));
    let report = Validator::new(false).validate_bytes(&data);
    let late: Vec<&Diagnostic> = report
        .errors()
        .filter(|d| d.rule_id == "metadata/hdr-cll-first-coded-picture")
        .collect();
    assert!(
        !late.is_empty(),
        "a unit targeting an established layer plus a new late layer must fire for the \
         new layer; report was: {report}"
    );
    assert!(
        late.iter().all(|d| d.message.contains("obu_mlayer_id 1"))
            && late.iter().all(|d| !d.message.contains("obu_mlayer_id 0")),
        "the finding must name only the new late layer (obu_mlayer_id 1), not the \
         established layer 0; report was: {report}"
    );
}

#[test]
fn metadata_hdr_cll_layer_values_repeat_for_established_layer_only_is_unchanged() {
    // Control for finding 4: a later unit targeting ONLY the already-established
    // layer (0,0) is an allowed repeat — the per-pair gate filters that pair out, so
    // no first-coded-picture finding fires (repeat semantics unchanged). Identical
    // content keeps the repeat-content check silent too.
    let mut data = td_and_seq_header(0, 0, 1);
    data.extend(local_hdr_cll_group_layer_values_obu(0b0000_0001, 1000, 200)); // baseline (0,0)
    data.extend(clk_frame_at_mlayer(0)); // layer (0,0) first picture
    data.extend(local_hdr_cll_group_layer_values_obu(0b0000_0001, 1000, 200)); // repeat (0,0)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-cll-first-coded-picture"),
        "an identical repeat for an already-established layer must not fire the \
         first-coded-picture finding; report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-cll-repeat-content-differs"),
        "an identical repeat must not fire repeat-content-differs; report was: {report}"
    );
}

#[test]
fn metadata_hdr_cll_layer_values_timely_for_all_targeted_layers_is_silent() {
    // The complement: a LAYER_VALUES HDR CLL indicated BEFORE either targeted
    // layer's first coded picture (in the pre-frame region) is timely for both and
    // must be silent.
    let mut data = td_and_seq_header(0, 0, 1);
    data.extend(local_hdr_cll_group_layer_values_obu(0b0000_0011, 1000, 200)); // before any picture
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // layer (0,0) first picture
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "metadata/hdr-cll-first-coded-picture"),
        "a LAYER_VALUES unit at/before the first picture of every targeted layer must \
         be silent; report was: {report}"
    );
}

// --- task 5.1: QM/FGM SEF false-negative regression ---

#[test]
fn qm_duplicate_level_across_sef_is_not_flagged() {
    // AV2 § 6.12: the duplicate-level rule is scoped "between coded frames" and
    // `QmSeen` to levels seen "since the last frame". § 7.3.3 lists a SEF as one of
    // the two alternatives for "the coded frame" of a unit and calls it a "frame"
    // ("Such a frame is associated with ... OrderHint"), so a SEF is a coded-frame
    // boundary that resets the QM window. A level repeated on either side of a SEF
    // is in two different coded frame units and is not a duplicate.
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(qm_default_level_obu(0));
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // SEF: resets the window
    data.extend(qm_default_level_obu(0)); // same level 0, new coded frame unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "qm/duplicate-level-between-frames"),
        "a SEF is a coded-frame boundary that resets the QM window: {report}"
    );
}

#[test]
fn qm_duplicate_level_across_decoded_frame_is_not_flagged() {
    // The complement: a decoded coded frame (a tile group) DOES reset the window,
    // so the same level on either side of it is not a duplicate. The RTG carries
    // a frame_header_present_flag == 0 (a header-copy tile group), enough to be a
    // frame-bearing OBU that resets the window.
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(qm_default_level_obu(0));
    data.extend(annex_b_obu(REGULAR_TILE_GROUP_HEADER, &[0x00, 0x80])); // decoded frame: resets
    data.extend(qm_default_level_obu(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "qm/duplicate-level-between-frames"),
        "a decoded coded frame resets the QM window; report was: {report}"
    );
}

#[test]
fn film_grain_duplicate_slot_across_sef_is_not_flagged() {
    // AV2 § 6.13: the duplicate-slot rule is scoped to the "same coded frame unit"
    // and its NOTE permits reuse "in a subsequent coded frame unit". § 7.3.3 makes
    // a single SEF its own coded frame unit, so a SEF ends the film-grain
    // coded-frame-unit window. A slot updated on either side of a SEF is in two
    // different coded frame units and must be allowed.
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(film_grain_obu_bytes(0b0000_0001, 0)); // slot 0
    data.extend(annex_b_obu(REGULAR_SEF_HEADER, &[0x80])); // SEF: closes the window
    data.extend(film_grain_obu_bytes(0b0000_0001, 0)); // slot 0, new coded frame unit
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "film-grain/duplicate-slot-in-coded-frame-unit"),
        "a SEF is its own coded frame unit, so a slot reused across it is allowed: {report}"
    );
}

/// A `metadata_hdr_cll()` OBU at xlayer 0 / `mlayer` with `LAYER_CURRENT`
/// targeting (muh_layer_idc == 1), carrying the given `max_cll` / `max_fall`.
pub(in crate::validator::tests) fn hdr_cll_unit_layer_current(
    mlayer: u8,
    max_cll: u32,
    max_fall: u32,
) -> Vec<u8> {
    // metadata_short header byte: metadata_is_suffix(1)=0, muh_layer_idc(3)=2
    // (LAYER_CURRENT), muh_cancel_flag(1)=0, muh_persistence_idc(3)=0.
    // 0b0_010_0_000 = 0x20.
    let mut unit = Bits::default();
    unit.f(max_cll, 16); // max_cll
    unit.f(max_fall, 16); // max_fall
    let payload = metadata_short_payload(0x20, 1, &unit.into_bytes()); // type 1 = HDR_CLL
    annex_b_obu_with_header(&layer_obu_header(8, 0, mlayer, 0), &payload)
}

/// The suffix form of [`hdr_cll_unit_layer_current`]: identical except
/// `metadata_is_suffix == 1` (header byte 0b1_010_0_000 = 0xA0), so § 7.3.3 places
/// it in the suffix-metadata tail of its coded frame unit, after the coded frame.
pub(in crate::validator::tests) fn hdr_cll_unit_layer_current_suffix(
    mlayer: u8,
    max_cll: u32,
    max_fall: u32,
) -> Vec<u8> {
    let mut unit = Bits::default();
    unit.f(max_cll, 16); // max_cll
    unit.f(max_fall, 16); // max_fall
    let payload = metadata_short_payload(0xA0, 1, &unit.into_bytes()); // suffix, type 1 = HDR_CLL
    annex_b_obu_with_header(&layer_obu_header(8, 0, mlayer, 0), &payload)
}
