// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn metadata_hdr_cll_first_coded_picture_late_is_flagged() {
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
    let mut data = td_and_seq_header(0, 0, 0);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // first coded picture
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
    let mut data = td_and_seq_header(0, 1, 0); // max_tlayer_id == 1
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // tlayer 0: first coded picture
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
    let mut data = td_and_seq_header(0, 0, 1); // max_mlayer_id = 1 allows mlayers 0 and 1
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // layer (0,0) first picture
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
    annex_b_obu_with_header(&layer_obu_header(4, 0, mlayer, 0), &bits.into_bytes())
}

#[test]
fn metadata_hdr_cll_layer_values_new_layer_late_past_established_layer_is_flagged() {
    let mut data = td_and_seq_header(0, 0, 1); // mlayers 0 and 1 valid
    data.extend(local_hdr_cll_group_layer_values_obu(0b0000_0001, 1000, 200));
    data.extend(clk_frame_at_mlayer(0)); // layer (0,0) first picture
    data.extend(clk_frame_at_mlayer(1)); // layer (0,1) first picture
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

#[test]
fn qm_duplicate_level_across_sef_is_not_flagged() {
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
