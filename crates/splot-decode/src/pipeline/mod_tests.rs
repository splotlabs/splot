// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::bitstream::byte_stream::{FlatParsedBitstream, prepare_byte_stream};
use crate::test_support::empty_avmenc_ivf;
use crate::{DecodeContext, DecodeRuntimeConfig};
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_parallel::ThreadCount;

const MULTIPLE_TILE_GROUP_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-2tile-2group-intra-128x64-q80.ivf"
);
const DEFAULT_QM_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-qm-intra-64x64.ivf");
const OUTPUT_EFFECT_CI_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-output-ci-2frame-64x64.ivf");
const DEFAULT_QM_AVM_DIGEST: &str =
    "a3a64b8df33017ea9c6c54b94bc54f6694e7ccb5e00edade7d00208de09a14b4";

const OBU_SEQUENCE_HEADER: u8 = 0x04;
const OBU_TEMPORAL_DELIMITER: u8 = 0x08;
const OBU_CLOSED_LOOP_KEY: u8 = 0x10;
const OBU_REGULAR_TILE_GROUP: u8 = 0x1C;
const OBU_REGULAR_TIP: u8 = 0x38;
const OBU_OPERATING_POINT_SET: u8 = 0x48;
const OBU_FILM_GRAIN: u8 = 0x5C;
const OBU_RESERVED_26: u8 = 0x68;

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

fn recorded_header(byte: u8, bit_len: u64) -> splot_core::Result<RecordedFrameHeaderBits> {
    let bytes = [byte];
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    RecordedFrameHeaderBits::record(&mut reader, bit_len)
}

fn long_term_frame(
    obu_type: ObuType,
    long_term_id: u32,
    hidden: bool,
    frame_index: usize,
) -> InBandLongTermFrame {
    InBandLongTermFrame {
        obu_type,
        long_term_id,
        hidden,
        frame_index,
    }
}

fn unsupported_reason<T>(result: Result<T>) -> Option<&'static str> {
    match result {
        Err(DecodeError::UnsupportedFeature { unsupported }) => Some(unsupported.reason()),
        _ => None,
    }
}

#[test]
fn every_output_adapter_rejects_invalid_frame_effects_in_the_shared_pipeline()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut bytes = OUTPUT_EFFECT_CI_FIXTURE.to_vec();
    let payload_offset = match parse_bitstream_partial(&bytes) {
        ParsedBitstream::Ivf(ivf) => ivf
            .frames
            .iter()
            .flat_map(|frame| &frame.obus)
            .find(|obu| obu.header.obu_type == ObuType::ContentInterpretation)
            .and_then(|obu| usize::try_from(obu.payload_offset().get()).ok()),
        ParsedBitstream::AnnexB(_) => None,
    };
    let payload_offset = payload_offset.ok_or("fixture lacks content interpretation")?;
    bytes[payload_offset] |= 0b01;
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))?;

    let results = [
        (
            "hash",
            context
                .decode_hash_report_bytes(&bytes, DecodeOptions::default())
                .map(drop),
        ),
        (
            "raw",
            context.decode_raw_bytes(&bytes, DecodeOptions::default(), Vec::new()),
        ),
        (
            "y4m",
            context.decode_y4m_bytes(&bytes, DecodeOptions::default(), Vec::new()),
        ),
    ];

    for (adapter, result) in results {
        assert_eq!(
            unsupported_reason(result),
            Some("content_interpretation_reserved_bits"),
            "{adapter} output bypassed the shared effect validation"
        );
    }
    Ok(())
}

#[test]
fn in_band_long_term_prelude_accepts_hidden_clk_then_olk_in_sequential_slots() {
    let prelude = InBandLongTermPrelude {
        frames: vec![
            long_term_frame(ObuType::ClosedLoopKey, 3, true, 4),
            long_term_frame(ObuType::OpenLoopKey, 7, true, 6),
        ],
    };

    let result = prelude.validate_required_with(&[3, 7], ByteOffset::new(20), |id, index| {
        (id, index) == (3, 4) || (id, index) == (7, 6)
    });

    assert!(result.is_ok());
}

#[test]
fn in_band_long_term_prelude_rejects_missing_or_visible_reference() {
    let missing = InBandLongTermPrelude::default().validate_required_with(
        &[3],
        ByteOffset::new(20),
        |_, _| true,
    );
    assert_eq!(
        unsupported_reason(missing),
        Some("random_access_long_term_reference_missing")
    );

    let visible = InBandLongTermPrelude {
        frames: vec![long_term_frame(ObuType::ClosedLoopKey, 3, false, 4)],
    }
    .validate_required_with(&[3], ByteOffset::new(20), |_, _| true);
    assert_eq!(
        unsupported_reason(visible),
        Some("random_access_long_term_reference_visible")
    );
}

#[test]
fn in_band_long_term_prelude_rejects_overwritten_slot_and_clk_after_olk() {
    let overwritten = InBandLongTermPrelude {
        frames: vec![long_term_frame(ObuType::ClosedLoopKey, 3, true, 4)],
    }
    .validate_required_with(&[3], ByteOffset::new(20), |_, _| false);
    assert_eq!(
        unsupported_reason(overwritten),
        Some("random_access_long_term_reference_slot_unavailable")
    );

    let wrong_order = InBandLongTermPrelude {
        frames: vec![
            long_term_frame(ObuType::OpenLoopKey, 7, true, 6),
            long_term_frame(ObuType::ClosedLoopKey, 3, true, 4),
        ],
    }
    .validate_required_with(&[3, 7], ByteOffset::new(20), |_, _| true);
    assert_eq!(
        unsupported_reason(wrong_order),
        Some("random_access_long_term_reference_order")
    );
}

#[test]
fn in_band_long_term_prelude_does_not_cross_temporal_unit_boundary() {
    let mut prelude = InBandLongTermPrelude {
        frames: vec![long_term_frame(ObuType::ClosedLoopKey, 3, true, 4)],
    };

    prelude.begin_frame(true);

    let result = prelude.validate_required_with(&[3], ByteOffset::new(20), |_, _| true);
    assert_eq!(
        unsupported_reason(result),
        Some("random_access_long_term_reference_missing")
    );
}

#[test]
fn continuation_prefix_locates_structure_with_and_without_header_copy()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let recorded = recorded_header(0xa0, 4)?;
    let no_copy = [0x02, OBU_CLOSED_LOOP_KEY, 0x00];
    let no_copy_envelope = annexb_obus(&no_copy)[0];
    assert!(matches!(
        continuation_structure_start_bits(no_copy_envelope, &recorded),
        Ok(2)
    ));

    let matching_copy = [0x02, OBU_CLOSED_LOOP_KEY, 0x68];
    let matching_copy_envelope = annexb_obus(&matching_copy)[0];
    assert!(matches!(
        continuation_structure_start_bits(matching_copy_envelope, &recorded),
        Ok(6)
    ));
    Ok(())
}

#[test]
fn continuation_prefix_rejects_mismatch_and_eof_in_header_copy()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let one_bit_header = recorded_header(0x80, 1)?;
    let mismatch = [0x02, OBU_CLOSED_LOOP_KEY, 0x40];
    let mismatch_envelope = annexb_obus(&mismatch)[0];
    assert!(continuation_structure_start_bits(mismatch_envelope, &one_bit_header).is_err());

    let byte_header = recorded_header(0x80, 8)?;
    let truncated = [0x02, OBU_CLOSED_LOOP_KEY, 0x60];
    let truncated_envelope = annexb_obus(&truncated)[0];
    assert!(continuation_structure_start_bits(truncated_envelope, &byte_header).is_err());
    Ok(())
}

#[test]
fn multiple_tile_groups_decode_bit_exact() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let options = DecodeOptions::default();
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))?;
    let plan = context.plan_bytes(MULTIPLE_TILE_GROUP_FIXTURE, options)?;
    assert_eq!(plan.frame_candidate_count(), 1);
    let decoded = context
        .pool()
        .install(|| decode_frame_from_plan(MULTIPLE_TILE_GROUP_FIXTURE, &options, &plan))?;
    let PipelineDecodedFrame::Eight(frame) = decoded.ready_frame()? else {
        return Err("fixture decoded as 10-bit".into());
    };
    assert_eq!(frame.y().samples().len(), 128 * 64);
    assert!(frame.y().samples().iter().all(|&sample| sample == 126));
    assert!(matches!(
        frame.u(),
        Some(plane) if plane.samples().iter().all(|&sample| sample == 128)
    ));
    assert!(matches!(
        frame.v(),
        Some(plane) if plane.samples().iter().all(|&sample| sample == 128)
    ));
    Ok(())
}

#[test]
fn absent_user_qm_data_uses_built_in_matrix() -> std::result::Result<(), Box<dyn std::error::Error>>
{
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))?;
    let report = context.decode_hash_report_bytes(DEFAULT_QM_FIXTURE, DecodeOptions::default())?;

    assert_eq!(report.frames.len(), 1);
    assert_eq!(report.frames[0].hashes[0].digest_hex, DEFAULT_QM_AVM_DIGEST);
    Ok(())
}

#[test]
fn empty_ivf_decodes_to_empty_frame_set() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let bytes = empty_avmenc_ivf();
    let options = DecodeOptions::default();
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))?;
    let plan = context.plan_bytes(&bytes, options)?;

    assert_eq!(plan.obu_count(), 0);
    assert_eq!(plan.frame_candidate_count(), 0);
    let frames = context
        .pool()
        .install(|| decode_frames_from_plan(&bytes, &options, &plan))?;
    assert!(frames.is_empty());
    Ok(())
}

#[test]
fn prepared_byte_stream_discards_reserved_obus_from_annex_b_and_ivf()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let reserved_0 = [0x02, 0x00, 0x80];
    let reserved_26 = [0x02, OBU_RESERVED_26, 0x80];
    let payload = [
        reserved_0.as_slice(),
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        reserved_26.as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
    ]
    .concat();

    let options = DecodeOptions::default();
    let annex_b = prepare_byte_stream(&payload, &options)?;
    assert_eq!(annex_b.plan().obu_count(), 5);
    assert!(matches!(annex_b.parsed(), FlatParsedBitstream::AnnexB(_)));
    let FlatParsedBitstream::AnnexB(annex_b) = annex_b.parsed() else {
        return Err("unexpected prepared bitstream format".into());
    };
    assert_eq!(annex_b.obus.len(), 3);
    assert!(
        annex_b
            .obus
            .iter()
            .all(|obu| !obu.header.obu_type.is_reserved())
    );

    let mut ivf_bytes = Vec::new();
    write_ivf_header(&mut ivf_bytes, &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 1))?;
    write_ivf_frame(&mut ivf_bytes, 0, &payload)?;
    let ivf = prepare_byte_stream(&ivf_bytes, &options)?;
    assert_eq!(ivf.plan().obu_count(), 5);
    assert!(matches!(ivf.parsed(), FlatParsedBitstream::Ivf(_)));
    let FlatParsedBitstream::Ivf(ivf) = ivf.parsed() else {
        return Err("unexpected prepared bitstream format".into());
    };
    assert_eq!(ivf.frames.len(), 1);
    assert_eq!(ivf.frame_obus(&ivf.frames[0]).len(), 3);
    assert!(
        ivf.frame_obus(&ivf.frames[0])
            .iter()
            .all(|obu| !obu.header.obu_type.is_reserved())
    );
    Ok(())
}

#[test]
fn leading_frame_unit_allows_ops_before_sequence()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_OPERATING_POINT_SET).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_REGULAR_TILE_GROUP).as_slice(),
    ]
    .concat();
    let obus = annexb_obus(&bytes);

    let ([td, sequence, key], frame_unit_len) = require_leading_frame_unit(&obus)?;

    assert_eq!(td.header.obu_type, ObuType::TemporalDelimiter);
    assert_eq!(sequence.header.obu_type, ObuType::SequenceHeader);
    assert_eq!(key.header.obu_type, ObuType::ClosedLoopKey);
    assert_eq!(frame_unit_len, 4);
    assert_eq!(leading_record_inter_frame_unit_start(0, 4, &obus), Some(4));
    assert!(require_leading_ivf_obu_order(&obus).is_ok());
    Ok(())
}

#[test]
fn inter_frame_unit_order_accepts_regular_tip_with_optional_film_grain() {
    let without_film_grain = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_REGULAR_TIP).as_slice(),
    ]
    .concat();
    let with_film_grain = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_FILM_GRAIN).as_slice(),
        obu(OBU_REGULAR_TIP).as_slice(),
    ]
    .concat();

    assert!(require_inter_obu_order(&annexb_obus(&without_film_grain)).is_ok());
    assert!(require_inter_obu_order(&annexb_obus(&with_film_grain)).is_ok());
}

#[test]
fn leading_frame_unit_allows_film_grain_before_key_and_inter()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_OPERATING_POINT_SET).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_FILM_GRAIN).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_FILM_GRAIN).as_slice(),
        obu(OBU_REGULAR_TILE_GROUP).as_slice(),
    ]
    .concat();
    let obus = annexb_obus(&bytes);

    let ([td, sequence, key], frame_unit_len) = require_leading_frame_unit(&obus)?;

    assert_eq!(td.header.obu_type, ObuType::TemporalDelimiter);
    assert_eq!(sequence.header.obu_type, ObuType::SequenceHeader);
    let leading_prefix = leading_prefix_obus(&obus)?;
    assert_eq!(leading_prefix.len(), 4);
    assert_eq!(leading_prefix[2].offset, sequence.offset);
    assert_eq!(leading_prefix[3].header.obu_type, ObuType::FilmGrain);
    assert_eq!(key.header.obu_type, ObuType::ClosedLoopKey);
    assert_eq!(frame_unit_len, 5);
    assert_eq!(leading_record_inter_frame_unit_start(0, 6, &obus), Some(5));
    assert!(require_leading_ivf_obu_order(&obus).is_ok());
    Ok(())
}

#[test]
fn leading_annexb_regular_after_key_stops_at_next_temporal_delimiter()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_FILM_GRAIN).as_slice(),
        obu(OBU_REGULAR_TILE_GROUP).as_slice(),
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_REGULAR_TILE_GROUP).as_slice(),
    ]
    .concat();
    let obus = annexb_obus(&bytes);
    let (_, frame_unit_len) = require_leading_frame_unit(&obus)?;
    let mut next_unvalidated = frame_unit_len;

    assert_eq!(frame_unit_len, 3);
    assert_eq!(leading_record_inter_frame_unit_start(0, 4, &obus), Some(3));
    assert_eq!(leading_record_inter_frame_unit_start(0, 6, &obus), None);

    assert!(require_following_annexb_obu_order_through(&obus, &mut next_unvalidated, 4).is_ok());
    assert_eq!(next_unvalidated, 5);
    assert!(require_following_annexb_obu_order_through(&obus, &mut next_unvalidated, 6).is_ok());
    assert_eq!(next_unvalidated, 7);
    Ok(())
}
