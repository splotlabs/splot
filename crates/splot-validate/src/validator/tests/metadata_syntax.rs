// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// Builds a `metadata_short_obu()` payload: the 1-byte header, a 1-byte metadata
/// type, the metadata unit bytes, and one OBU trailing byte.
pub(in crate::validator::tests) fn metadata_short_payload(
    first: u8,
    metadata_type: u8,
    unit: &[u8],
) -> Vec<u8> {
    let mut payload = vec![first, metadata_type];
    payload.extend_from_slice(unit);
    payload.push(0x80);
    payload
}

/// A global `OBU_METADATA_SHORT` (xlayer 31) after a temporal delimiter.
pub(in crate::validator::tests) fn global_metadata_short_stream(payload: &[u8]) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(8, 0, 0, 31),
        payload,
    ));
    data
}

/// A global `OBU_METADATA_GROUP` (xlayer 31) after a temporal delimiter.
pub(in crate::validator::tests) fn global_metadata_group_stream(payload: &[u8]) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(9, 0, 0, 31),
        payload,
    ));
    data
}

#[test]
fn metadata_short_layer_idc_out_of_range_is_flagged() {
    // first byte 0x38 = 0b0_011_1_000: layer_idc=3 (>= 3), cancel=1.
    let payload = [0x38, 0x04, 0x80];
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        has_error(&report, "metadata/short-layer-idc-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn metadata_short_payload_underflow_is_flagged() {
    // obuPayloadSize = 2, leb128 bytes = 1 -> 2 - 2 - 1 underflows.
    let payload = [0x00, 0x01];
    let report = Validator::new(false).validate_bytes(&global_metadata_short_stream(&payload));
    assert!(
        has_error(&report, "metadata/unit-payload-underflow"),
        "report was: {report}"
    );
}

#[test]
fn metadata_group_unit_count_too_large_is_flagged() {
    // metadata_unit_cnt_minus_1 = 16383 (leb128 0xFF 0x7F).
    let payload = [0x00, 0xFF, 0x7F, 0x80];
    let report = Validator::new(false).validate_bytes(&global_metadata_group_stream(&payload));
    assert!(
        has_error(&report, "metadata/group-unit-count-too-large"),
        "report was: {report}"
    );
}

#[test]
fn metadata_group_header_underflow_is_flagged() {
    // Non-cancel unit, muh_header_size = 0: the payload_size leb byte alone makes
    // headerRemainingBytes negative.
    let payload = [0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x80];
    let report = Validator::new(false).validate_bytes(&global_metadata_group_stream(&payload));
    assert!(
        has_error(&report, "metadata/group-header-underflow"),
        "report was: {report}"
    );
}

#[test]
fn metadata_group_reserved_bits_nonzero_is_warned() {
    // type=0 (UnknownRaw), header_size=3, payload_size=0, reserved bits = 0b01.
    let payload = [0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x01, 0x80];
    let report = Validator::new(false).validate_bytes(&global_metadata_group_stream(&payload));
    assert!(
        has_warning(&report, "metadata/group-reserved-bits-nonzero"),
        "report was: {report}"
    );
}

#[test]
fn metadata_group_xlayer_map_global_bit_set_is_flagged() {
    // Global group, layer_idc=LAYER_VALUES, muh_xlayer_map = 0x8000_0000 (bit 31 set).
    // header_size = payload_size leb (1) + fixed 2 + 4 (xlayer_map) = 7.
    let payload = [
        0x00, 0x00, // group header + cnt
        0x00, // metadata_type = 0
        0x0E, // muh_header_size = 7, cancel = 0
        0x00, // muh_payload_size = 0
        0x60, 0x00, // layer_idc=LAYER_VALUES(3), persistence=0, priority=0, reserved=0
        0x80, 0x00, 0x00, 0x00, // muh_xlayer_map = bit 31 set
        0x80, // OBU trailing byte
    ];
    let report = Validator::new(false).validate_bytes(&global_metadata_group_stream(&payload));
    assert!(
        has_error(&report, "metadata/group-xlayer-map-global-bit-set"),
        "report was: {report}"
    );
}

#[test]
fn metadata_group_mlayer_map_below_obu_mlayer_is_flagged() {
    // Local group at mlayer 1, layer_idc=LAYER_VALUES -> one muh_mlayer_map byte with
    // bit 0 set (below obu_mlayer_id = 1). header_size = leb(1) + 2 + 1 = 4.
    let payload = [
        0x00, 0x00, // group header + cnt
        0x00, // metadata_type = 0
        0x08, // muh_header_size = 4, cancel = 0
        0x00, // muh_payload_size = 0
        0x60, 0x00, // layer_idc=LAYER_VALUES(3)
        0x01, // muh_mlayer_map = bit 0 set
        0x80, // OBU trailing byte
    ];
    // A non-global metadata OBU needs an active sequence header for its xlayer.
    let mut data = temporal_delimiter_obu();
    data.extend(sequence_header_obu_for_xlayer(2, 1, 1));
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(9, 0, 1, 2),
        &payload,
    ));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "metadata/group-mlayer-map-below-obu-mlayer"),
        "report was: {report}"
    );
}

#[test]
fn metadata_temporal_point_info_in_group_is_flagged() {
    // A group unit with metadata_type = METADATA_TYPE_TEMPORAL_POINT_INFO (9).
    let payload = [0x00, 0x00, 0x09, 0x01, 0x80]; // one cancelled unit, type 9
    let report = Validator::new(false).validate_bytes(&global_metadata_group_stream(&payload));
    assert!(
        has_error(&report, "metadata/temporal-point-info-not-short"),
        "report was: {report}"
    );
}
