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
    use splot_core::annexb::parse_annex_b_obus;
    use splot_core::types::ExtendedLayerId;

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
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(0, 0, 0)));
    data.extend(annex_b_obu(0x04, &sequence_header_payload_with_id(1, 1, 0)));
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
