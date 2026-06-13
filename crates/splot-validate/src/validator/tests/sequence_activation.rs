// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn frame_header_seq_header_id_out_of_range_is_not_double_reported() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 16)); // == MAX_SEQ_NUM -> out of range
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/seq-header-id-out-of-range"),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
        "an out-of-range id must not also report unavailable; report was: {report}"
    );
}

#[test]
fn frame_header_cur_mfh_id_out_of_range_is_not_double_reported() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 16)); // == MAX_MFH_NUM -> out of range
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "frame-header/cur-mfh-id-out-of-range"),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-multi-frame-header"),
        "an out-of-range cur_mfh_id must not also report unavailable; report was: {report}"
    );
}

#[test]
fn sequence_activation_uses_clk_referenced_sequence_header() {
    // Two available sequence headers with different layer limits: id 0 allows
    // tlayer up to 2, id 1 allows only tlayer 0. A CLK that references id 1
    // activates it for xlayer 0, so a following tlayer-1 OBU exceeds the limit.
    // Without frame-header activation, id 0 (the OBU-order fallback) would be
    // active and the tlayer-1 OBU would be accepted.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 2, 2)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 0)));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 1)); // activate id 1
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 0, 0), &[]));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
        "the CLK-referenced sequence header (id 1) must bound the tlayer; report was: {report}"
    );
}

#[test]
fn sequence_fingerprint_preserved_for_in_cvs_repeat() {
    // A sequence header opens a CVS, a CLK references (activates) it, then a
    // non-identical repeat of the same id appears later in the SAME temporal
    // unit. Per AV2 § 7.3.6 the new coded video sequence starts at the temporal
    // unit, so the pre-CLK header joins it: its fingerprint survives the
    // activating CLK and the same-temporal-unit repeat is flagged eagerly.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0)); // activates id 0
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
        "report was: {report}"
    );
}

#[test]
fn sequence_reconfiguration_in_clk_temporal_unit_is_not_flagged() {
    // Temporal unit 2 reconfigures id 0 with different layer limits and contains
    // a CLK *after* the header: AV2 § 7.3.6 defines the new coded video sequence
    // to start at the temporal unit ("A new coded video sequence ... is defined
    // to start at each temporal unit that contains an OBU with obu_type equal to
    // OBU_CLOSED_LOOP_KEY ..."), so the pre-CLK params-B header joins the NEW
    // coded video sequence and is never in the same sequence as params A. The
    // deferred cross-temporal-unit comparison enqueued when params B was
    // observed is dropped when the CLK arrives later in the same temporal unit
    // (no false positive).
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 1, 1)));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));
    data.extend(temporal_delimiter_obu());
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 0));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/repeated-sequence-header-not-identical"),
        "a reconfiguration in a CLK temporal unit must not be flagged; report was: {report}"
    );
}

#[test]
fn first_picture_in_tu_is_tracked_per_extended_layer() {
    // AV2 § 6.17.2: "FirstPictureInTU is a variable that specifies if this is
    // the first frame unit in a coded extended layer unit in a temporal unit" —
    // i.e. per extended layer. A frame-bearing OBU in xlayer 0 must not clear
    // xlayer 1's FirstPictureInTU (so a CLK for xlayer 1 later in the same
    // temporal unit still derives startCVS, AV2 § 5.18.2), and the next global
    // temporal delimiter resets the per-temporal-unit state. The derivation is
    // not observable through diagnostics yet (startCVS gates no implemented
    // check), so this drives the context directly.
    use splot_core::annexb::parse_annex_b_obus;
    use splot_core::types::ExtendedLayerId;

    // TD; leading tile group (type 6) in xlayer 0; CLK (type 4) in xlayer 1; TD.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));
    data.extend(annex_b_obu_with_header(&layer_obu_header(4, 0, 0, 1), &[]));
    data.extend(temporal_delimiter_obu());

    let obus = parse_annex_b_obus(&data).unwrap_or_default();
    assert_eq!(obus.len(), 4, "the test stream must parse into 4 OBUs");

    let options = ValidationOptions::default();
    let mut report = ValidationReport::new();
    let mut context = ValidatorContext::default();
    let x0 = ExtendedLayerId::from_bits(0);
    let x1 = ExtendedLayerId::from_bits(1);

    context.observe_obu(&obus[0], &options, &mut report); // temporal delimiter
    assert!(context.first_picture_in_tu(x0));
    assert!(context.first_picture_in_tu(x1));

    context.observe_obu(&obus[1], &options, &mut report); // frame in xlayer 0
    assert!(!context.first_picture_in_tu(x0));
    assert!(
        context.first_picture_in_tu(x1),
        "a frame in xlayer 0 must not clear xlayer 1's FirstPictureInTU"
    );

    context.observe_obu(&obus[2], &options, &mut report); // CLK in xlayer 1
    assert!(!context.first_picture_in_tu(x1));

    context.observe_obu(&obus[3], &options, &mut report); // next temporal unit
    assert!(context.first_picture_in_tu(x0));
    assert!(context.first_picture_in_tu(x1));
}

/// A frame-bearing OBU (with an extension header at the given layer ids) whose
/// first tile group carries a frame header referencing `seq_header_id`.
pub(in crate::validator::tests) fn frame_obu_direct_seq_ref_layer(
    obu_type: u8,
    tlayer: u8,
    mlayer: u8,
    xlayer: u8,
    seq_header_id: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(1); // is_first_tile_group
    bits.uvlc(0); // cur_mfh_id == 0
    bits.uvlc(seq_header_id); // seq_header_id_in_frame_header
    annex_b_obu_with_header(
        &layer_obu_header(obu_type, tlayer, mlayer, xlayer),
        &bits.into_bytes(),
    )
}

/// A multi-frame header OBU with in-range ids but a malformed §5.2.1 payload tail
/// (`obu_extension_flag == 1`), so it is not a valid available HLS object.
pub(in crate::validator::tests) fn malformed_tail_mfh_obu(
    mfh_id_minus_1: u32,
    seq_header_id: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.uvlc(seq_header_id); // mfh_seq_header_id
    bits.uvlc(mfh_id_minus_1); // mfh_id_minus_1
    bits.bit(0); // mfh_frame_size_present_flag
    bits.bit(0); // mfh_deblocking_filter_update
    bits.bit(0); // mfh_seg_info_present_flag -> fully parsed
    bits.bit(1); // obu_extension_flag = 1 -> §6.2.1 tail violation
    annex_b_obu(0x0C, &bits.into_bytes())
}

#[test]
fn frame_header_activation_precedes_layer_limit_check() {
    // A CLK requires obu_tlayer_id == 0 but may carry a non-zero obu_mlayer_id
    // (AV2 §6.2.2). seq 0 allows only mlayer 0; seq 1 allows mlayer 1. A CLK at
    // obu_mlayer_id 1 that references seq 1 activates the permissive header BEFORE
    // its own layer-limit check, so it is not flagged. (Without activating first,
    // the stale seq-0 fallback would falsely flag mlayer-exceeds-max.)
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 0, 1)));
    data.extend(frame_obu_direct_seq_ref_layer(4, 0, 1, 0, 1)); // CLK, mlayer 1, ref seq 1

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/mlayer-exceeds-max"),
        "the CLK must activate seq 1 (allows mlayer 1) before its own limit check; \
         report was: {report}"
    );
}

#[test]
fn frame_header_referencing_malformed_tail_mfh_is_unavailable() {
    // An MFH with in-range ids but a malformed §5.2.1 payload tail is not recorded
    // as available, so a frame referencing it via cur_mfh_id is unavailable rather
    // than resolved through the malformed HLS object.
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(malformed_tail_mfh_obu(1, 0)); // mfhId 2, malformed tail
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 2)); // CLK cur_mfh_id 2
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-multi-frame-header"),
        "report was: {report}"
    );
}

#[test]
fn frame_header_missing_mfh_under_external_hls_is_not_flagged() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    // With external HLS provided, an out-of-band multi-frame header may satisfy the
    // cur_mfh_id reference. External MFHs are not modeled, so the validator neither
    // resolves the MFH nor emits a hard error — it must not reject the conformant
    // external-HLS stream.
    let mut data = temporal_delimiter_obu();
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 2)); // CLK cur_mfh_id 2, no in-band MFH
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(0)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-multi-frame-header"),
        "external HLS may supply the MFH; report was: {report}"
    );
}

#[test]
fn frame_header_activation_applies_to_non_key_frames() {
    // AV2 §5.18.2 calls load_sequence_header() for every frame, before the
    // `if (keyFrame)` block — not just CLK/OLK key frames. seq 0 allows only
    // tlayer 0; seq 1 allows tlayer 1. A non-key OBU_REGULAR_TILE_GROUP at tlayer 1
    // that references seq 1 activates it, so it is checked against seq 1 (allows
    // tlayer 1) rather than the stale seq-0 fallback — no false positive.
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 0)));
    // OBU_REGULAR_TILE_GROUP (type 7), tlayer 1, mlayer 0, xlayer 0, references seq 1.
    data.extend(frame_obu_direct_seq_ref_layer(7, 1, 0, 0, 1));

    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
        "a non-key frame must activate its referenced (permissive) seq header; \
         report was: {report}"
    );
}
