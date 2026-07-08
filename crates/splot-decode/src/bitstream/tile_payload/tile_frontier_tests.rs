// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode, FrameReferenceStateView,
    parse_frame_header_core,
};
use splot_core::headers::sequence::{SequenceHeader, parse_sequence_header};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_parallel::ThreadCount;

use super::super::{
    DecodeTileWorkUnit, FrameCandidateCdfFacts, FrameCandidateCoeffFacts,
    FrameCandidateTileBoundaryInput, FrameCandidateTileFacts, TileGroupPositionFacts,
    plan_derived_tile_payload_boundary,
};
use super::*;
use crate::{
    DecodeContext, DecodeLimitName, DecodeLimitThreshold, DecodeOptions, DecodeRuntimeConfig,
};

const LEGACY_INVERTED_SKIP_TRACE: &[u8] = &[
    0x44, 0x4b, 0x49, 0x46, 0x00, 0x00, 0x20, 0x00, 0x41, 0x56, 0x30, 0x32, 0x40, 0x00, 0x40, 0x00,
    0x1e, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x16, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x08, 0x0c, 0x04,
    0x82, 0x0a, 0x55, 0xff, 0xf1, 0xc2, 0x0d, 0xd7, 0x0b, 0x70, 0x11, 0x06, 0x10, 0xe3, 0xfe, 0x21,
    0x2f, 0xba,
];

#[test]
fn legacy_inverted_skip_trace_fails_closed_before_output() {
    with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
        let Err(err) =
            plan_tile_partition_frontier(work_unit, sequence, core, DecodeLimits::DEFAULT)
        else {
            panic!("retired payload unexpectedly reached the runtime frontier")
        };

        assert!(
            matches!(
                err,
                TilePartitionFrontierError::UnexpectedFrontier {
                    reason: "unexpected_decode_block_frontier"
                }
            ),
            "expected the retired payload to fail closed at the runtime frontier, got {err:?}"
        );
    });
}

#[test]
fn partition_frontier_checks_padded_mi_state_cells_before_allocation() {
    with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
        let mut padded_core = core.clone();
        let tile_info = padded_core.tile_info.as_mut().unwrap();
        *tile_info.mi_row_starts.last_mut().unwrap() = 17;
        *tile_info.mi_col_starts.last_mut().unwrap() = 16;
        let limits = DecodeLimits::unlimited()
            .with_max_luma_samples_per_frame(DecodeLimitThreshold::Max(300));

        let Err(err) = plan_tile_partition_frontier(work_unit, sequence, &padded_core, limits)
        else {
            panic!("expected padded MI-state limit");
        };

        let TilePartitionFrontierError::Limit(limit) = err else {
            panic!("expected padded MI-state limit, got {err:?}");
        };
        assert_eq!(limit.name(), DecodeLimitName::MaxLumaSamplesPerFrame);
        assert_eq!(limit.actual(), Some(512));
    });
}

#[test]
fn partition_frontier_checks_mi_state_byte_budget_before_allocation() {
    with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
        let limits =
            DecodeLimits::unlimited().with_max_decoded_frame_bytes(DecodeLimitThreshold::Max(1024));

        let Err(err) = plan_tile_partition_frontier(work_unit, sequence, core, limits) else {
            panic!("expected MI-state byte limit");
        };

        let TilePartitionFrontierError::Limit(limit) = err else {
            panic!("expected MI-state byte limit, got {err:?}");
        };
        assert_eq!(limit.name(), DecodeLimitName::MaxDecodedFrameBytes);
        assert_eq!(
            limit.actual(),
            Some((2 * (256 + 16 + 16) * size_of::<usize>()) as u64)
        );
    });
}

#[test]
fn lr_unit_frontier_checks_padded_mi_state_cells_before_allocation() {
    with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
        let mut padded_core = core.clone();
        let tile_info = padded_core.tile_info.as_mut().unwrap();
        *tile_info.mi_row_starts.last_mut().unwrap() = 17;
        *tile_info.mi_col_starts.last_mut().unwrap() = 16;
        let limits = DecodeLimits::unlimited()
            .with_max_luma_samples_per_frame(DecodeLimitThreshold::Max(300));

        let Err(err) = consume_tile_lr_unit_frontier(work_unit, sequence, &padded_core, limits)
        else {
            panic!("expected padded MI-state limit");
        };

        let TilePartitionFrontierError::Limit(limit) = err else {
            panic!("expected padded MI-state limit, got {err:?}");
        };
        assert_eq!(limit.name(), DecodeLimitName::MaxLumaSamplesPerFrame);
        assert_eq!(limit.actual(), Some(512));
    });
}

#[test]
fn lr_unit_frontier_checks_mi_state_byte_budget_before_allocation() {
    with_minimal_work_unit(LEGACY_INVERTED_SKIP_TRACE, |work_unit, sequence, core| {
        let limits =
            DecodeLimits::unlimited().with_max_decoded_frame_bytes(DecodeLimitThreshold::Max(1024));

        let Err(err) = consume_tile_lr_unit_frontier(work_unit, sequence, core, limits) else {
            panic!("expected MI-state byte limit");
        };

        let TilePartitionFrontierError::Limit(limit) = err else {
            panic!("expected MI-state byte limit, got {err:?}");
        };
        assert_eq!(limit.name(), DecodeLimitName::MaxDecodedFrameBytes);
        assert_eq!(
            limit.actual(),
            Some((2 * (256 + 16 + 16) * size_of::<usize>()) as u64)
        );
    });
}

fn with_minimal_work_unit<R>(
    bytes: &[u8],
    f: impl FnOnce(&mut DecodeTileWorkUnit<'_>, &SequenceHeader, &FrameHeaderCore) -> R + Send,
) -> R
where
    R: Send,
{
    let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
    context.pool().install(move || {
        let context =
            DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();
        let plan = context.plan_bytes(bytes, DecodeOptions::default()).unwrap();
        let candidate = plan.frame_candidates().next().unwrap();

        let ParsedBitstream::Ivf(ivf) = parse_bitstream_partial(bytes) else {
            panic!("minimal fixture must parse as IVF");
        };
        let frame = ivf.frames.first().unwrap();
        let [_, sequence_envelope, frame_envelope] = frame.obus.as_slice() else {
            panic!("minimal fixture must contain temporal delimiter, sequence, and frame OBUs");
        };

        let mut sequence_reader = BitReader::new(
            sequence_envelope.payload,
            sequence_envelope.payload_offset(),
        );
        let sequence = parse_sequence_header(&mut sequence_reader).unwrap();

        let mut frame_reader =
            BitReader::new(frame_envelope.payload, frame_envelope.payload_offset());
        assert_ne!(frame_reader.read_bit().unwrap(), 0);
        let frame_input = FrameHeaderParseInput {
            obu_type: frame_envelope.header.obu_type,
            first_picture_in_tu: true,
            active_sequence: Some(&sequence),
            mfh_record: None,
            reference_state: FrameReferenceStateView::unknown(),
            mode: FrameHeaderParseMode::Core,
        };
        let core = parse_frame_header_core(&mut frame_reader, &frame_input).unwrap();

        let tq = sequence.transform_quant_entropy.as_ref().unwrap();
        let coeff = FrameCandidateCoeffFacts::from_tq(tq);
        let facts = FrameCandidateTileFacts::from_frame_core(&core, coeff).unwrap();
        let cdf = FrameCandidateCdfFacts::new(tq.enable_avg_cdf, tq.avg_cdf_type != 0);
        let input = FrameCandidateTileBoundaryInput::new(
            &plan,
            candidate,
            bytes,
            *frame_envelope,
            TileGroupPositionFacts::new(true, true),
            facts,
            cdf,
            DecodeLimits::DEFAULT,
        );
        let mut tile_plan = plan_derived_tile_payload_boundary(&input).unwrap();
        let [work_unit] = tile_plan.work_units_mut() else {
            panic!("minimal fixture must derive one tile work unit");
        };

        f(work_unit, &sequence, &core)
    })
}
