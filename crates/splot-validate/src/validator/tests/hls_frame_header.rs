// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// A frame-bearing OBU (`header` byte) whose first tile group carries a frame
/// header with `cur_mfh_id == 0` and the given `seq_header_id_in_frame_header`.
pub(in crate::validator::tests) fn frame_obu_direct_seq_ref(
    header: u8,
    seq_header_id: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(1); // is_first_tile_group -> frame_header_present_flag inferred 1
    bits.uvlc(0); // cur_mfh_id == 0 -> direct sequence-header reference
    bits.uvlc(seq_header_id); // seq_header_id_in_frame_header
    annex_b_obu(header, &bits.into_bytes())
}

/// A frame-bearing OBU whose first tile group carries a frame header with
/// `cur_mfh_id` greater than 0 (the sequence header resolves through the MFH).
pub(in crate::validator::tests) fn frame_obu_mfh_ref(header: u8, cur_mfh_id: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.bit(1); // is_first_tile_group
    bits.uvlc(cur_mfh_id); // cur_mfh_id > 0
    annex_b_obu(header, &bits.into_bytes())
}

/// Temporal delimiter + an activating sequence header (id `seq_id`) for xlayer 0.
pub(in crate::validator::tests) fn td_and_seq_header(
    seq_id: u32,
    max_tlayer_id: u32,
    max_mlayer_id: u32,
) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_id(seq_id, max_tlayer_id, max_mlayer_id),
    ));
    data
}

pub(in crate::validator::tests) const CLK_HEADER: u8 = 0x10;

#[test]
fn hls_frame_header_missing_sequence_header_is_flagged() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 5)); // references missing id 5
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
        "report was: {report}"
    );
}

#[test]
fn hls_frame_header_sequence_header_available_inband_is_accepted() {
    let mut data = td_and_seq_header(3, 1, 1);
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 3)); // references available id 3
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
        "report was: {report}"
    );
}

#[test]
fn hls_frame_header_sequence_header_available_external_is_accepted() {
    use crate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
    let mut data = temporal_delimiter_obu();
    data.extend(frame_obu_direct_seq_ref(CLK_HEADER, 5));
    let options = ValidationOptions {
        external_hls: ExternalHlsMode::Provided(ExternalHlsSet::new().with_sequence_header_id(5)),
    };
    let report = Validator::new(false).validate_bytes_with_options(&data, &options);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
        "report was: {report}"
    );
}

#[test]
fn hls_frame_header_missing_mfh_is_flagged() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 2)); // references missing MFH id 2
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-multi-frame-header"),
        "report was: {report}"
    );
}

#[test]
fn hls_frame_header_mfh_available_is_accepted() {
    let mut data = td_and_seq_header(0, 1, 1);
    data.extend(multi_frame_header_obu(0)); // mfh_seq_header_id 0 -> mfhId 1
    data.extend(frame_obu_mfh_ref(CLK_HEADER, 1)); // resolves MFH 1 -> seq 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-multi-frame-header"),
        "report was: {report}"
    );
    assert!(
        !report
            .errors()
            .any(|d| d.rule_id == "hls/unavailable-sequence-header"),
        "report was: {report}"
    );
}

pub(in crate::validator::tests) const LEADING_TILE_GROUP_HEADER: u8 = 0x18;
pub(in crate::validator::tests) const REGULAR_TILE_GROUP_HEADER: u8 = 0x1C;

/// A global temporal delimiter followed by a sequence header with `seq_header_id`
/// for xlayer 0 (no embedded/temporal layering).
pub(in crate::validator::tests) fn td_and_seq(seq_header_id: u32) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(
        0x04,
        &sequence_header_payload_with_id(seq_header_id, 1, 1),
    ));
    data
}

/// A bare sequence header OBU with `seq_header_id` for xlayer 0 (a resend within an
/// already-open temporal unit).
pub(in crate::validator::tests) fn seq_obu(seq_header_id: u32) -> Vec<u8> {
    annex_b_obu(0x04, &sequence_header_payload_with_id(seq_header_id, 1, 1))
}

/// A bare sequence header OBU with `seq_header_id` carried in extended layer `xlayer`
/// (a resend by a specific layer; the `seq_header_id` namespace is global, § 7.3.8.6).
pub(in crate::validator::tests) fn seq_obu_layer(seq_header_id: u32, xlayer: u8) -> Vec<u8> {
    annex_b_obu_with_header(
        &layer_obu_header(1, 0, 0, xlayer),
        &sequence_header_payload_with_id(seq_header_id, 1, 1),
    )
}

pub(in crate::validator::tests) const RAP_RULE: &str = "hls/unavailable-at-random-access-point";
