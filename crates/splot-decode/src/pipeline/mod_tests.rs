// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use splot_core::stream::parse_bitstream_partial;

const OBU_SEQUENCE_HEADER: u8 = 0x04;
const OBU_TEMPORAL_DELIMITER: u8 = 0x08;
const OBU_CLOSED_LOOP_KEY: u8 = 0x10;
const OBU_REGULAR_TILE_GROUP: u8 = 0x1C;
const OBU_REGULAR_TIP: u8 = 0x38;
const OBU_OPERATING_POINT_SET: u8 = 0x48;
const OBU_FILM_GRAIN: u8 = 0x5C;

fn obu(header: u8) -> [u8; 2] {
    [0x01, header]
}

fn annexb_obus(bytes: &[u8]) -> Vec<ObuEnvelope<'_>> {
    let parsed = parse_bitstream_partial(bytes);
    assert!(matches!(parsed, ParsedBitstream::AnnexB(_)));
    let ParsedBitstream::AnnexB(parsed) = parsed else {
        return Vec::new();
    };
    assert!(parsed.error.is_none());
    parsed.obus
}

#[test]
fn leading_frame_unit_allows_ops_before_sequence() {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_OPERATING_POINT_SET).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_REGULAR_TILE_GROUP).as_slice(),
    ]
    .concat();
    let obus = annexb_obus(&bytes);

    let leading = require_leading_frame_unit(&obus);
    assert!(leading.is_ok());
    let Ok(([td, sequence, key], frame_unit_len)) = leading else {
        return;
    };

    assert_eq!(td.header.obu_type, ObuType::TemporalDelimiter);
    assert_eq!(sequence.header.obu_type, ObuType::SequenceHeader);
    assert_eq!(key.header.obu_type, ObuType::ClosedLoopKey);
    assert_eq!(frame_unit_len, 4);
    assert!(is_leading_record_regular_after_key(0, 4, &obus));
    assert!(require_leading_ivf_obu_order(&obus).is_ok());
}

#[test]
fn inter_frame_unit_order_accepts_regular_tip() {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_REGULAR_TIP).as_slice(),
    ]
    .concat();
    let obus = annexb_obus(&bytes);

    assert!(require_inter_obu_order(&obus).is_ok());
}

#[test]
fn leading_frame_unit_allows_film_grain_before_key() {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_OPERATING_POINT_SET).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_FILM_GRAIN).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_REGULAR_TILE_GROUP).as_slice(),
    ]
    .concat();
    let obus = annexb_obus(&bytes);

    let leading = require_leading_frame_unit(&obus);
    assert!(leading.is_ok());
    let Ok(([td, sequence, key], frame_unit_len)) = leading else {
        return;
    };

    assert_eq!(td.header.obu_type, ObuType::TemporalDelimiter);
    assert_eq!(sequence.header.obu_type, ObuType::SequenceHeader);
    let film_grain_result = leading_film_grain_obus(&obus);
    assert!(film_grain_result.is_ok());
    let Ok(film_grain_obus) = film_grain_result else {
        return;
    };
    assert_eq!(film_grain_obus.len(), 1);
    assert_eq!(key.header.obu_type, ObuType::ClosedLoopKey);
    assert_eq!(frame_unit_len, 5);
    assert!(is_leading_record_regular_after_key(0, 5, &obus));
    assert!(require_leading_ivf_obu_order(&obus).is_ok());
}

#[test]
fn leading_annexb_regular_after_key_stops_at_next_temporal_delimiter() {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_REGULAR_TILE_GROUP).as_slice(),
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_REGULAR_TILE_GROUP).as_slice(),
    ]
    .concat();
    let obus = annexb_obus(&bytes);
    let leading = require_leading_frame_unit(&obus);
    assert!(leading.is_ok());
    let Ok((_, frame_unit_len)) = leading else {
        return;
    };
    let mut next_unvalidated = frame_unit_len;

    assert_eq!(frame_unit_len, 3);
    assert!(is_leading_record_regular_after_key(0, 3, &obus));
    assert!(!is_leading_record_regular_after_key(0, 5, &obus));

    assert!(require_following_annexb_obu_order_through(&obus, &mut next_unvalidated, 3).is_ok());
    assert_eq!(next_unvalidated, 4);
    assert!(require_following_annexb_obu_order_through(&obus, &mut next_unvalidated, 5).is_ok());
    assert_eq!(next_unvalidated, 6);
}
