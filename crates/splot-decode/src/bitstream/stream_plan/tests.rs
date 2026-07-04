// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::{DecodeContext, DecodeLimitThreshold, DecodeLimits, DecodeRuntimeConfig};
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use splot_core::stream::parse_bitstream_partial;
use splot_parallel::ThreadCount;

const OBU_SEQUENCE_HEADER: u8 = 0x04;
const OBU_TEMPORAL_DELIMITER: u8 = 0x08;
const OBU_CLOSED_LOOP_KEY: u8 = 0x10;
const OBU_OPEN_LOOP_KEY: u8 = 0x14;
const OBU_REGULAR_TILE_GROUP: u8 = 0x1C;
const OBU_REGULAR_TIP: u8 = 0x38;
const OBU_METADATA_SHORT: u8 = 0x20;
const OBU_MSDO: u8 = 0x50;
const OBU_PADDING: u8 = 0x64;

fn obu(header: u8) -> [u8; 2] {
    [0x01, header]
}

fn extended_obu(header: u8, extension: u8) -> [u8; 3] {
    [0x02, header, extension]
}

fn context(threads: ThreadCount) -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(threads)).unwrap()
}

fn plan(bytes: &[u8]) -> DecodeStreamPlan {
    let parsed = parse_bitstream_partial(bytes);
    context(ThreadCount::from(1usize))
        .plan_stream(
            DecodeStreamInput::new(&parsed, bytes.len() as u64),
            DecodeOptions::default(),
        )
        .unwrap()
}

fn plan_bytes(bytes: &[u8]) -> DecodeStreamPlan {
    context(ThreadCount::from(1usize))
        .plan_bytes(bytes, DecodeOptions::default())
        .unwrap()
}

fn ivf_with_payloads(payloads: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_ivf_header(
        &mut bytes,
        &IvfHeader::new(*b"AV02", 16, 16, 24, 1, payloads.len() as u32),
    )
    .unwrap();
    for (index, payload) in payloads.iter().enumerate() {
        write_ivf_frame(&mut bytes, index as u64 * 10, payload).unwrap();
    }
    bytes
}

#[test]
fn raw_annex_b_plan_preserves_order_and_roles() {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_PADDING).as_slice(),
    ]
    .concat();

    let plan = plan(&bytes);
    let obus: Vec<_> = plan.obus().collect();

    assert_eq!(plan.format(), BitstreamFormat::AnnexB);
    assert_eq!(plan.selected_layer(), DecodeLayerSelection::base());
    assert_eq!(plan.input_len_bytes(), bytes.len() as u64);
    assert_eq!(plan.obu_count(), 4);
    assert_eq!(plan.frame_candidate_count(), 1);
    assert_eq!(plan.frame_candidates().count(), 1);
    assert_eq!(obus[0].index(), 0);
    assert_eq!(obus[0].offset(), ByteOffset::new(1));
    assert_eq!(obus[0].role(), DecodePlannedObuRole::Global);
    assert_eq!(obus[1].role(), DecodePlannedObuRole::SelectedLayerState);
    assert_eq!(obus[2].role(), DecodePlannedObuRole::FrameCandidate);
    assert_eq!(obus[2].payload_len(), 0);
    assert_eq!(obus[3].role(), DecodePlannedObuRole::Global);
    assert!(
        obus.iter()
            .all(|obu| obu.source_kind() == DecodeObuSourceKind::AnnexB)
    );
    assert!(obus.iter().all(|obu| obu.ivf_frame().is_none()));
}

#[test]
fn regular_tile_group_is_admitted_as_inter_frame_candidate() {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_REGULAR_TILE_GROUP).as_slice(),
    ]
    .concat();

    let plan = plan(&bytes);
    let obus: Vec<_> = plan.obus().collect();

    assert_eq!(plan_bytes(&bytes), plan);
    assert_eq!(plan.frame_candidate_count(), 2);
    assert_eq!(plan.frame_candidates().count(), 1);
    assert_eq!(plan.frame_candidates_all().count(), 2);
    assert_eq!(obus[2].role(), DecodePlannedObuRole::FrameCandidate);
    assert_eq!(obus[4].role(), DecodePlannedObuRole::InterFrameCandidate);
    assert!(obus[2].role().is_frame_candidate());
    assert!(obus[4].role().is_frame_candidate());
}

#[test]
fn regular_tip_is_admitted_as_inter_frame_candidate() {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_REGULAR_TIP).as_slice(),
    ]
    .concat();

    let plan = plan(&bytes);
    let obus: Vec<_> = plan.obus().collect();

    assert_eq!(plan_bytes(&bytes), plan);
    assert_eq!(plan.frame_candidate_count(), 2);
    assert_eq!(plan.frame_candidates().count(), 1);
    assert_eq!(plan.frame_candidates_all().count(), 2);
    assert_eq!(obus[2].role(), DecodePlannedObuRole::FrameCandidate);
    assert_eq!(obus[4].role(), DecodePlannedObuRole::InterFrameCandidate);
    assert!(obus[4].role().is_frame_candidate());
}

#[test]
fn byte_planner_raw_annex_b_matches_parsed_plan() {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_PADDING).as_slice(),
    ]
    .concat();

    assert_eq!(plan_bytes(&bytes), plan(&bytes));
}

#[test]
fn ivf_plan_preserves_frame_context_and_warning_metadata() {
    let first = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
    ]
    .concat();
    let second = obu(OBU_CLOSED_LOOP_KEY);
    let mut bytes = ivf_with_payloads(&[&first, &second]);
    bytes.push(0xAA);

    let plan = plan_bytes(&bytes);
    let obus: Vec<_> = plan.obus().collect();

    assert_eq!(plan.format(), BitstreamFormat::Ivf);
    assert_eq!(plan.source_warnings().len(), 1);
    assert_eq!(
        plan.source_warnings()[0].kind(),
        DecodeSourceIssueKind::IvfWarning
    );
    assert_eq!(obus.len(), 3);
    assert_eq!(obus[0].source_kind(), DecodeObuSourceKind::Ivf);
    let first_context = obus[0].ivf_frame().unwrap();
    assert_eq!(first_context.frame_index(), 0);
    assert_eq!(first_context.pts(), 0);
    assert_eq!(first_context.frame_payload_offset(), ByteOffset::new(44));
    assert_eq!(obus[0].offset(), ByteOffset::new(45));
    let second_context = obus[2].ivf_frame().unwrap();
    assert_eq!(second_context.frame_index(), 1);
    assert_eq!(second_context.pts(), 10);

    let parsed_plan = {
        let parsed = parse_bitstream_partial(&bytes);
        context(ThreadCount::from(1usize))
            .plan_stream(
                DecodeStreamInput::new(&parsed, bytes.len() as u64),
                DecodeOptions::default(),
            )
            .unwrap()
    };
    assert_eq!(plan, parsed_plan);
}

#[test]
fn malformed_raw_source_is_transactional() {
    let bytes = [0x01, OBU_TEMPORAL_DELIMITER, 0x05, OBU_CLOSED_LOOP_KEY];
    let parsed = parse_bitstream_partial(&bytes);

    let error = context(ThreadCount::from(1usize))
        .plan_stream(
            DecodeStreamInput::new(&parsed, bytes.len() as u64),
            DecodeOptions::default(),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == DecodeSourceIssueKind::AnnexBParseError
            && issue.offset() == Some(ByteOffset::new(3))
    ));
}

#[test]
fn byte_planner_malformed_sources_are_transactional() {
    let raw = [0x01, OBU_TEMPORAL_DELIMITER, 0x05, OBU_CLOSED_LOOP_KEY];
    let raw_error = context(ThreadCount::from(1usize))
        .plan_bytes(&raw, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(
        raw_error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == DecodeSourceIssueKind::AnnexBParseError
            && issue.offset() == Some(ByteOffset::new(3))
    ));

    let truncated_header = b"DKIF";
    let container_error = context(ThreadCount::from(1usize))
        .plan_bytes(truncated_header, DecodeOptions::default())
        .unwrap_err();
    assert!(matches!(
        container_error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == DecodeSourceIssueKind::IvfContainerError
    ));

    let ivf = ivf_with_payloads(&[&[0x05, OBU_CLOSED_LOOP_KEY]]);
    let frame_error = context(ThreadCount::from(1usize))
        .plan_bytes(&ivf, DecodeOptions::default())
        .unwrap_err();
    assert!(matches!(
        frame_error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == DecodeSourceIssueKind::IvfFramePayloadError
            && issue.frame_index() == Some(0)
    ));
}

#[test]
fn byte_planner_ivf_eof_branches_are_transactional() {
    let mut truncated_frame_header = Vec::new();
    write_ivf_header(
        &mut truncated_frame_header,
        &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 1),
    )
    .unwrap();
    truncated_frame_header.push(0xAA);
    let frame_header_error = context(ThreadCount::from(1usize))
        .plan_bytes(&truncated_frame_header, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(
        frame_header_error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == DecodeSourceIssueKind::IvfContainerError
            && issue.frame_index() == Some(0)
    ));

    let mut truncated_payload = Vec::new();
    write_ivf_header(
        &mut truncated_payload,
        &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 1),
    )
    .unwrap();
    truncated_payload.extend_from_slice(&4u32.to_le_bytes());
    truncated_payload.extend_from_slice(&0u64.to_le_bytes());
    truncated_payload.push(0x01);

    let payload_error = context(ThreadCount::from(1usize))
        .plan_bytes(&truncated_payload, DecodeOptions::default())
        .unwrap_err();
    assert!(matches!(
        payload_error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == DecodeSourceIssueKind::IvfContainerError
            && issue.frame_index() == Some(0)
    ));
}

#[test]
fn malformed_ivf_container_and_frame_payload_are_transactional() {
    let truncated_header = b"DKIF";
    let parsed_container = parse_bitstream_partial(truncated_header);
    let container_error = context(ThreadCount::from(1usize))
        .plan_stream(
            DecodeStreamInput::new(&parsed_container, truncated_header.len() as u64),
            DecodeOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(
        container_error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == DecodeSourceIssueKind::IvfContainerError
    ));

    let bytes = ivf_with_payloads(&[&[0x05, OBU_CLOSED_LOOP_KEY]]);
    let parsed_frame = parse_bitstream_partial(&bytes);
    let frame_error = context(ThreadCount::from(1usize))
        .plan_stream(
            DecodeStreamInput::new(&parsed_frame, bytes.len() as u64),
            DecodeOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(
        frame_error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == DecodeSourceIssueKind::IvfFramePayloadError
            && issue.frame_index() == Some(0)
    ));
}

#[test]
fn malformed_later_ivf_payload_wins_over_earlier_unsupported_obu() {
    let unsupported = obu(OBU_OPEN_LOOP_KEY);
    let malformed = [0x05, OBU_CLOSED_LOOP_KEY];
    let bytes = ivf_with_payloads(&[&unsupported, &malformed]);

    let parsed = parse_bitstream_partial(&bytes);
    let parsed_error = context(ThreadCount::from(1usize))
        .plan_stream(
            DecodeStreamInput::new(&parsed, bytes.len() as u64),
            DecodeOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(
        parsed_error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == DecodeSourceIssueKind::IvfFramePayloadError
            && issue.frame_index() == Some(1)
    ));

    let error = context(ThreadCount::from(1usize))
        .plan_bytes(&bytes, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::MalformedSource {
            issue
        } if issue.kind() == DecodeSourceIssueKind::IvfFramePayloadError
            && issue.frame_index() == Some(1)
    ));
}

#[test]
fn parsed_ivf_obu_limits_win_before_later_payload_errors() {
    let accepted = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
    ]
    .concat();
    let malformed = [0x05, OBU_CLOSED_LOOP_KEY];
    let bytes = ivf_with_payloads(&[&accepted, &malformed]);
    let parsed = parse_bitstream_partial(&bytes);

    let obu_error = context(ThreadCount::from(1usize))
        .plan_stream(
            DecodeStreamInput::new(&parsed, bytes.len() as u64),
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_obus(DecodeLimitThreshold::Max(1)),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        obu_error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxObus
    ));

    let frame_candidates = [
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
    ]
    .concat();
    let bytes = ivf_with_payloads(&[&frame_candidates, &malformed]);
    let parsed = parse_bitstream_partial(&bytes);
    let frame_error = context(ThreadCount::from(1usize))
        .plan_stream(
            DecodeStreamInput::new(&parsed, bytes.len() as u64),
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_frames_to_decode(DecodeLimitThreshold::Max(1)),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        frame_error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxFramesToDecode
    ));
}

#[test]
fn byte_ivf_record_limit_wins_over_earlier_unsupported_obu() {
    let unsupported = obu(OBU_OPEN_LOOP_KEY);
    let second = obu(OBU_TEMPORAL_DELIMITER);
    let bytes = ivf_with_payloads(&[&unsupported, &second]);

    let error = context(ThreadCount::from(1usize))
        .plan_bytes(
            &bytes,
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_ivf_frame_records(DecodeLimitThreshold::Max(1)),
            ),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxIvfFrameRecords
    ));
}

#[test]
fn local_limits_reject_input_obus_ivf_records_and_frame_candidates() {
    let bytes = [
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
    ]
    .concat();
    let parsed = parse_bitstream_partial(&bytes);
    let ctx = context(ThreadCount::from(1usize));

    let input_error = ctx
        .plan_stream(
            DecodeStreamInput::new(&parsed, bytes.len() as u64),
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_input_bytes(DecodeLimitThreshold::Max(1)),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        input_error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxInputBytes
    ));

    let obu_error = ctx
        .plan_stream(
            DecodeStreamInput::new(&parsed, bytes.len() as u64),
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_obus(DecodeLimitThreshold::Max(2)),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        obu_error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxObus
    ));

    let ivf_bytes = ivf_with_payloads(&[&[], &[]]);
    let parsed_ivf = parse_bitstream_partial(&ivf_bytes);
    let ivf_record_error = ctx
        .plan_stream(
            DecodeStreamInput::new(&parsed_ivf, ivf_bytes.len() as u64),
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_ivf_frame_records(DecodeLimitThreshold::Max(1)),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        ivf_record_error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxIvfFrameRecords
    ));

    let frame_error = ctx
        .plan_stream(
            DecodeStreamInput::new(&parsed, bytes.len() as u64),
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_frames_to_decode(DecodeLimitThreshold::Max(1)),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        frame_error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxFramesToDecode
    ));
}

#[test]
fn byte_planner_limits_reject_before_unbounded_traversal() {
    let ctx = context(ThreadCount::from(1usize));
    let malformed_second_obu = [0x01, OBU_TEMPORAL_DELIMITER, 0x05, OBU_CLOSED_LOOP_KEY];

    let input_error = ctx
        .plan_bytes(
            &malformed_second_obu,
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_input_bytes(DecodeLimitThreshold::Max(1)),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        input_error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxInputBytes
    ));

    let obu_error = ctx
        .plan_bytes(
            &malformed_second_obu,
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_obus(DecodeLimitThreshold::Max(1)),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        obu_error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxObus
    ));

    let ivf = ivf_with_payloads(&[&[], &[0x05, OBU_CLOSED_LOOP_KEY]]);
    let ivf_record_error = ctx
        .plan_bytes(
            &ivf,
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_ivf_frame_records(DecodeLimitThreshold::Max(1)),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        ivf_record_error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxIvfFrameRecords
    ));

    let frames = [
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
        &[0x05, OBU_CLOSED_LOOP_KEY][..],
    ]
    .concat();
    let frame_error = ctx
        .plan_bytes(
            &frames,
            DecodeOptions::new(
                DecodeLimits::unlimited().with_max_frames_to_decode(DecodeLimitThreshold::Max(1)),
            ),
        )
        .unwrap_err();
    assert!(matches!(
        frame_error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxFramesToDecode
    ));
}

#[test]
fn unsupported_layers_and_obu_types_are_typed() {
    let cases = [
        (
            obu(OBU_OPEN_LOOP_KEY).to_vec(),
            DecodeUnsupportedReason::UnsupportedFrameObu,
        ),
        (
            obu(OBU_MSDO).to_vec(),
            DecodeUnsupportedReason::MultistreamSelection,
        ),
        (
            obu(OBU_METADATA_SHORT).to_vec(),
            DecodeUnsupportedReason::UnsupportedOutputEffectObu,
        ),
        (obu(0x00).to_vec(), DecodeUnsupportedReason::ReservedObu),
        (
            extended_obu(0x90, 0x20).to_vec(),
            DecodeUnsupportedReason::NonBaseEmbeddedLayer,
        ),
        (
            extended_obu(0x90, 0x01).to_vec(),
            DecodeUnsupportedReason::NonBaseExtendedLayer,
        ),
        (
            extended_obu(0x88, 0x00).to_vec(),
            DecodeUnsupportedReason::InvalidLayerScope,
        ),
        (
            extended_obu(0x90, 0x1F).to_vec(),
            DecodeUnsupportedReason::InvalidLayerScope,
        ),
        (
            extended_obu(0x84, 0x1F).to_vec(),
            DecodeUnsupportedReason::InvalidLayerScope,
        ),
        (
            obu(OBU_CLOSED_LOOP_KEY | 0x01).to_vec(),
            DecodeUnsupportedReason::NonBaseTemporalLayer,
        ),
    ];

    for (bytes, reason) in cases {
        let parsed = parse_bitstream_partial(&bytes);
        let error = context(ThreadCount::from(1usize))
            .plan_stream(
                DecodeStreamInput::new(&parsed, bytes.len() as u64),
                DecodeOptions::default(),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            DecodeError::UnsupportedStructure {
                unsupported
            } if unsupported.reason() == reason
                && unsupported.rule_id() == UNSUPPORTED_FEATURE_RULE_ID
                && unsupported.matrix_row() == DECODE_STREAM_STATE_MATRIX_ROW
                && unsupported.feature_id() == DECODE_STREAM_STATE_FEATURE_ID
        ));
    }
}

#[test]
fn byte_planner_propagates_unsupported_structures() {
    let cases = [
        (
            obu(OBU_OPEN_LOOP_KEY).to_vec(),
            DecodeUnsupportedReason::UnsupportedFrameObu,
        ),
        (
            obu(OBU_MSDO).to_vec(),
            DecodeUnsupportedReason::MultistreamSelection,
        ),
        (
            obu(OBU_METADATA_SHORT).to_vec(),
            DecodeUnsupportedReason::UnsupportedOutputEffectObu,
        ),
        (obu(0x00).to_vec(), DecodeUnsupportedReason::ReservedObu),
        (
            extended_obu(0x90, 0x20).to_vec(),
            DecodeUnsupportedReason::NonBaseEmbeddedLayer,
        ),
        (
            extended_obu(0x90, 0x01).to_vec(),
            DecodeUnsupportedReason::NonBaseExtendedLayer,
        ),
        (
            extended_obu(0x88, 0x00).to_vec(),
            DecodeUnsupportedReason::InvalidLayerScope,
        ),
        (
            extended_obu(0x90, 0x1F).to_vec(),
            DecodeUnsupportedReason::InvalidLayerScope,
        ),
        (
            extended_obu(0x84, 0x1F).to_vec(),
            DecodeUnsupportedReason::InvalidLayerScope,
        ),
        (
            obu(OBU_CLOSED_LOOP_KEY | 0x01).to_vec(),
            DecodeUnsupportedReason::NonBaseTemporalLayer,
        ),
    ];

    for (bytes, reason) in cases {
        let error = context(ThreadCount::from(1usize))
            .plan_bytes(&bytes, DecodeOptions::default())
            .unwrap_err();

        assert!(matches!(
            error,
            DecodeError::UnsupportedStructure {
                unsupported
            } if unsupported.reason() == reason
                && unsupported.rule_id() == UNSUPPORTED_FEATURE_RULE_ID
                && unsupported.matrix_row() == DECODE_STREAM_STATE_MATRIX_ROW
                && unsupported.feature_id() == DECODE_STREAM_STATE_FEATURE_ID
        ));
    }
}

#[test]
fn planning_is_deterministic_across_thread_policies() {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
    ]
    .concat();
    let parsed = parse_bitstream_partial(&bytes);
    let input = DecodeStreamInput::new(&parsed, bytes.len() as u64);

    let one = context(ThreadCount::from(1usize))
        .plan_stream(input, DecodeOptions::default())
        .unwrap();
    let auto = context(ThreadCount::Auto)
        .plan_stream(input, DecodeOptions::default())
        .unwrap();
    let fixed = context(ThreadCount::from(4usize))
        .plan_stream(input, DecodeOptions::default())
        .unwrap();

    assert_eq!(one, auto);
    assert_eq!(one, fixed);
}

#[test]
fn byte_planning_is_deterministic_across_thread_policies() {
    let bytes = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
        obu(OBU_CLOSED_LOOP_KEY).as_slice(),
    ]
    .concat();

    let one = context(ThreadCount::from(1usize))
        .plan_bytes(&bytes, DecodeOptions::default())
        .unwrap();
    let auto = context(ThreadCount::Auto)
        .plan_bytes(&bytes, DecodeOptions::default())
        .unwrap();
    let fixed = context(ThreadCount::from(4usize))
        .plan_bytes(&bytes, DecodeOptions::default())
        .unwrap();

    assert_eq!(one, auto);
    assert_eq!(one, fixed);
}

fn error_signature(error: DecodeError) -> String {
    match error {
        DecodeError::Pool { source } => format!("pool:{source}"),
        DecodeError::Limit { source } => format!("limit:{}:{source}", source.name()),
        DecodeError::MalformedSource { issue } => format!(
            "malformed:{:?}:{:?}:{:?}:{}",
            issue.kind(),
            issue.offset(),
            issue.frame_index(),
            issue.message()
        ),
        DecodeError::UnsupportedStructure { unsupported } => format!(
            "unsupported:{}:{}:{}",
            unsupported.reason(),
            unsupported.obu_type().spec_name(),
            unsupported.offset()
        ),
        DecodeError::UnsupportedFeature { unsupported } => format!(
            "unsupported-feature:{}:{}:{:?}",
            unsupported.reason(),
            unsupported.tier_id(),
            unsupported.byte_offset()
        ),
        DecodeError::Reconstruction { source } => format!("reconstruction:{source}"),
        DecodeError::Output { source } => format!(
            "output:{}:{}:{}",
            source.operation(),
            source.source_kind(),
            source.source_message()
        ),
    }
}

fn plan_bytes_error_signature(
    threads: ThreadCount,
    bytes: &[u8],
    options: &DecodeOptions,
) -> String {
    error_signature(context(threads).plan_bytes(bytes, *options).unwrap_err())
}

#[test]
fn byte_planning_errors_are_deterministic_across_thread_policies() {
    let malformed = [0x01, OBU_TEMPORAL_DELIMITER, 0x05, OBU_CLOSED_LOOP_KEY];
    let unsupported = obu(OBU_OPEN_LOOP_KEY);
    let limit_options =
        DecodeOptions::new(DecodeLimits::unlimited().with_max_obus(DecodeLimitThreshold::Max(1)));
    let limit = [
        obu(OBU_TEMPORAL_DELIMITER).as_slice(),
        obu(OBU_SEQUENCE_HEADER).as_slice(),
    ]
    .concat();

    for (bytes, options) in [
        (&malformed[..], DecodeOptions::default()),
        (&unsupported[..], DecodeOptions::default()),
        (&limit[..], limit_options),
    ] {
        let one = plan_bytes_error_signature(ThreadCount::from(1usize), bytes, &options);
        let auto = plan_bytes_error_signature(ThreadCount::Auto, bytes, &options);
        let fixed = plan_bytes_error_signature(ThreadCount::from(4usize), bytes, &options);

        assert_eq!(one, auto);
        assert_eq!(one, fixed);
    }
}
