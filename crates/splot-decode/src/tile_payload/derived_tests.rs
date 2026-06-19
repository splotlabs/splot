// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::input::{FrameCandidateTileMalformed, FrameCandidateTileUnsupportedReason};
use super::*;
use crate::{DecodeContext, DecodeLimitThreshold, DecodeOptions, DecodeRuntimeConfig};
use splot_core::annexb::{ObuEnvelope, parse_annex_b_obus};
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{
    FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode, FrameReferenceStateView,
    parse_frame_header_core,
};
use splot_core::headers::sequence::parse_sequence_header;
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::types::ObuType;
use splot_parallel::ThreadCount;

const OBU_CLOSED_LOOP_KEY_HEADER: u8 = 0x10;

fn context(threads: ThreadCount) -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(threads)).unwrap()
}

fn annex_b_tile_group_obu() -> Vec<u8> {
    annex_b_tile_group_obu_with_payload(&[0x00, 0x80, 0x00])
}

fn annex_b_tile_group_obu_with_payload(payload: &[u8]) -> Vec<u8> {
    let size = u8::try_from(payload.len() + 1).unwrap();
    let mut bytes = vec![size, OBU_CLOSED_LOOP_KEY_HEADER];
    bytes.extend_from_slice(payload);
    bytes
}

fn annex_b_envelope(bytes: &[u8]) -> ObuEnvelope<'_> {
    parse_annex_b_obus(bytes)
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
}

fn ivf_with_payload(payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_ivf_header(&mut bytes, &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 1)).unwrap();
    write_ivf_frame(&mut bytes, 0, payload).unwrap();
    bytes
}

fn ivf_envelope(bytes: &[u8]) -> ObuEnvelope<'_> {
    let ParsedBitstream::Ivf(parsed) = parse_bitstream_partial(bytes) else {
        panic!("expected ivf parse");
    };
    parsed.frames[0].obus[0]
}

fn base_candidate_facts(disable_cdf_update: bool) -> FrameCandidateTileFacts<'static> {
    FrameCandidateTileFacts::new_for_test(
        ObuType::ClosedLoopKey,
        true,
        false,
        1,
        1,
        &[0, 16],
        &[0, 8],
        None,
        0,
        42,
        disable_cdf_update,
    )
}

fn base_position() -> TileGroupPositionFacts {
    TileGroupPositionFacts::new(true, true)
}

fn base_cdf_facts() -> FrameCandidateCdfFacts {
    FrameCandidateCdfFacts::new(false, false)
}

fn base_coeff_facts() -> FrameCandidateCoeffFacts {
    FrameCandidateCoeffFacts::new(false, false)
}

fn derive_tile_payload_plan<'a>(
    ctx: &DecodeContext,
    bytes: &'a [u8],
    envelope: ObuEnvelope<'a>,
    position: TileGroupPositionFacts,
    facts: FrameCandidateTileFacts<'_>,
    limits: DecodeLimits,
) -> Result<DecodeTilePayloadPlan<'a>, FrameCandidateTileBoundaryError> {
    let stream_plan = ctx.plan_bytes(bytes, DecodeOptions::default()).unwrap();
    let candidate = stream_plan.frame_candidates().next().unwrap();
    ctx.plan_derived_tile_payload_boundary(FrameCandidateTileBoundaryInput::new(
        &stream_plan,
        candidate,
        bytes,
        envelope,
        position,
        facts,
        base_cdf_facts(),
        limits,
    ))
}

fn activation_only_frame_header_core() -> FrameHeaderCore {
    let data = [0xA0]; // uvlc(0) cur_mfh_id, uvlc(1) seq_header_id_in_frame_header.
    let mut reader = BitReader::new(&data, ByteOffset::new(0));
    let input = FrameHeaderParseInput {
        obu_type: ObuType::ClosedLoopKey,
        first_picture_in_tu: true,
        active_sequence: None,
        mfh_record: None,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).unwrap()
}

#[test]
fn derived_annex_b_tile_payload_preserves_source_offsets_and_boundary() {
    let bytes = annex_b_tile_group_obu();
    let ctx = context(ThreadCount::from(1usize));
    let envelope = annex_b_envelope(&bytes);
    let plan = derive_tile_payload_plan(
        &ctx,
        &bytes,
        envelope,
        base_position(),
        base_candidate_facts(false),
        DecodeLimits::unlimited(),
    )
    .unwrap();

    assert_eq!(plan.source().source_kind(), DecodeObuSourceKind::AnnexB);
    assert_eq!(plan.source().ivf_frame(), None);
    assert_eq!(plan.source().obu_index(), 0);
    assert_eq!(plan.source().obu_offset(), ByteOffset::new(1));
    assert_eq!(plan.selected_layer(), DecodeLayerSelection::base());
    assert_eq!(plan.work_units().len(), 1);
    let unit = &plan.work_units()[0];
    assert_eq!(unit.tile_bytes(), &[0x80, 0x00]);
    assert_eq!(unit.tile_byte_span(), ByteSpan::new(ByteOffset::new(3), 2));
    assert_eq!(unit.mi_row_range(), 0..8);
    assert_eq!(unit.mi_col_range(), 0..16);
    assert_eq!(unit.current_q_index_at_entry(), 42);
    assert_eq!(unit.symbol().cdf_update_mode(), CdfUpdateMode::Enabled);
    assert_eq!(unit.cdf().update_mode(), CdfUpdateMode::Enabled);
    assert_eq!(
        plan.unsupported().reason(),
        TilePayloadUnsupportedReason::DecodeTileSyntax
    );
    assert!(plan.frame_end().reaches_last_tile_group());
}

#[test]
fn derived_ivf_tile_payload_preserves_frame_context_and_offsets() {
    let frame_payload = annex_b_tile_group_obu();
    let bytes = ivf_with_payload(&frame_payload);
    let ctx = context(ThreadCount::from(1usize));
    let envelope = ivf_envelope(&bytes);
    let plan = derive_tile_payload_plan(
        &ctx,
        &bytes,
        envelope,
        base_position(),
        base_candidate_facts(false),
        DecodeLimits::unlimited(),
    )
    .unwrap();

    assert_eq!(plan.source().source_kind(), DecodeObuSourceKind::Ivf);
    assert_eq!(plan.source().obu_index(), 0);
    assert_eq!(plan.source().obu_offset(), ByteOffset::new(45));
    let frame = plan.source().ivf_frame().unwrap();
    assert_eq!(frame.frame_index(), 0);
    assert_eq!(frame.frame_header_offset(), ByteOffset::new(32));
    assert_eq!(frame.frame_payload_offset(), ByteOffset::new(44));
    assert_eq!(frame.frame_payload_size(), frame_payload.len() as u32);
    assert_eq!(
        plan.work_units()[0].tile_byte_span(),
        ByteSpan::new(ByteOffset::new(47), 2)
    );
    assert_eq!(plan.work_units()[0].tile_bytes(), &[0x80, 0x00]);
}

#[test]
fn derived_boundary_honors_disable_cdf_update_fact() {
    let bytes = annex_b_tile_group_obu();
    let ctx = context(ThreadCount::from(1usize));
    let envelope = annex_b_envelope(&bytes);
    let plan = derive_tile_payload_plan(
        &ctx,
        &bytes,
        envelope,
        base_position(),
        base_candidate_facts(true),
        DecodeLimits::unlimited(),
    )
    .unwrap();

    assert_eq!(
        plan.work_units()[0].symbol().cdf_update_mode(),
        CdfUpdateMode::Disabled
    );
    assert_eq!(
        plan.work_units()[0].cdf().update_mode(),
        CdfUpdateMode::Disabled
    );
}

#[test]
fn derived_boundary_honors_parser_derived_disable_cdf_update_fact() {
    let bytes = include_bytes!("../../../../tests/fixtures/frame-header-core.av2");
    let envelopes = parse_annex_b_obus(bytes).unwrap();
    let seq_envelope = envelopes[1];
    let frame_envelope = envelopes[2];
    let mut seq_reader = BitReader::new(seq_envelope.payload, seq_envelope.payload_offset());
    let sequence = parse_sequence_header(&mut seq_reader).unwrap();
    let mut frame_reader = BitReader::new(frame_envelope.payload, frame_envelope.payload_offset());
    assert_eq!(frame_reader.read_bit().unwrap(), 1);
    let input = FrameHeaderParseInput {
        obu_type: ObuType::ClosedLoopKey,
        first_picture_in_tu: false,
        active_sequence: Some(&sequence),
        mfh_record: None,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    let core = parse_frame_header_core(&mut frame_reader, &input).unwrap();
    let tq = sequence.transform_quant_entropy.as_ref().unwrap();
    let coeff = FrameCandidateCoeffFacts::new(tq.enable_fsc, tq.enable_chroma_dctonly);
    let facts = FrameCandidateTileFacts::from_frame_core(&core, coeff).unwrap();

    assert_eq!(core.disable_cdf_update, Some(false));
    let ctx = context(ThreadCount::from(1usize));
    let plan = derive_tile_payload_plan(
        &ctx,
        bytes,
        frame_envelope,
        base_position(),
        facts,
        DecodeLimits::unlimited(),
    )
    .unwrap();

    assert_eq!(plan.work_units().len(), 1);
    assert_eq!(
        plan.work_units()[0].symbol().cdf_update_mode(),
        CdfUpdateMode::Enabled
    );
    assert_eq!(
        plan.work_units()[0].cdf().update_mode(),
        CdfUpdateMode::Enabled
    );
}

#[test]
fn derived_boundary_threads_parser_coeff_frame_facts() {
    let bytes = include_bytes!("../../../../tests/fixtures/frame-header-core.av2");
    let envelopes = parse_annex_b_obus(bytes).unwrap();
    let seq_envelope = envelopes[1];
    let frame_envelope = envelopes[2];
    let mut seq_reader = BitReader::new(seq_envelope.payload, seq_envelope.payload_offset());
    let sequence = parse_sequence_header(&mut seq_reader).unwrap();
    let tq = sequence.transform_quant_entropy.as_ref().unwrap();
    let mut frame_reader = BitReader::new(frame_envelope.payload, frame_envelope.payload_offset());
    assert_eq!(frame_reader.read_bit().unwrap(), 1);
    let input = FrameHeaderParseInput {
        obu_type: ObuType::ClosedLoopKey,
        first_picture_in_tu: false,
        active_sequence: Some(&sequence),
        mfh_record: None,
        reference_state: FrameReferenceStateView::unknown(),
        mode: FrameHeaderParseMode::Core,
    };
    let core = parse_frame_header_core(&mut frame_reader, &input).unwrap();
    let coeff = FrameCandidateCoeffFacts::new(tq.enable_fsc, tq.enable_chroma_dctonly);
    let facts = FrameCandidateTileFacts::from_frame_core(&core, coeff).unwrap();
    let ctx = context(ThreadCount::from(1usize));
    let plan = derive_tile_payload_plan(
        &ctx,
        bytes,
        frame_envelope,
        base_position(),
        facts,
        DecodeLimits::unlimited(),
    )
    .unwrap();
    let coeff_facts = plan.work_units()[0].coeff_frame_facts();
    let lossless = core.lossless_info.unwrap();
    let tail = core.intra_tail.unwrap();
    let quant = core.quantization_params.unwrap();

    assert_eq!(coeff_facts.enable_fsc(), tq.enable_fsc);
    assert_eq!(
        coeff_facts.enable_chroma_dctonly(),
        tq.enable_chroma_dctonly
    );
    assert_eq!(
        coeff_facts.reduced_tx_set(),
        usize::from(tail.reduced_tx_set)
    );
    assert_eq!(coeff_facts.base_q_idx(), quant.base_q_idx);
    assert_eq!(coeff_facts.allow_tcq(), lossless.allow_tcq);
    assert_eq!(
        coeff_facts.allow_parity_hiding(),
        lossless.allow_parity_hiding
    );
    assert_eq!(
        coeff_facts.lossless_for_segment(0),
        Some(lossless.lossless_array[0])
    );
}

#[test]
fn derived_boundary_rejects_candidate_envelope_mismatch_before_slicing() {
    let bytes = annex_b_tile_group_obu();
    let ctx = context(ThreadCount::from(1usize));
    let stream_plan = ctx.plan_bytes(&bytes, DecodeOptions::default()).unwrap();
    let candidate = stream_plan.frame_candidates().next().unwrap();
    let mut envelope = annex_b_envelope(&bytes);
    envelope.size = envelope.size.saturating_add(1);

    let error = ctx
        .plan_derived_tile_payload_boundary(FrameCandidateTileBoundaryInput::new(
            &stream_plan,
            candidate,
            &bytes,
            envelope,
            base_position(),
            base_candidate_facts(false),
            base_cdf_facts(),
            DecodeLimits::unlimited(),
        ))
        .unwrap_err();

    let FrameCandidateTileBoundaryError::Malformed(
        FrameCandidateTileMalformed::CandidateEnvelopeMismatch { field },
    ) = error
    else {
        panic!("expected candidate/envelope mismatch");
    };
    assert_eq!(field, "size");
}

#[test]
fn derived_boundary_rejects_envelope_payload_from_different_input_buffer() {
    let bytes = annex_b_tile_group_obu();
    let forged_bytes = annex_b_tile_group_obu();
    let ctx = context(ThreadCount::from(1usize));
    let stream_plan = ctx.plan_bytes(&bytes, DecodeOptions::default()).unwrap();
    let candidate = stream_plan.frame_candidates().next().unwrap();
    let envelope = annex_b_envelope(&forged_bytes);

    let error = ctx
        .plan_derived_tile_payload_boundary(FrameCandidateTileBoundaryInput::new(
            &stream_plan,
            candidate,
            &bytes,
            envelope,
            base_position(),
            base_candidate_facts(false),
            base_cdf_facts(),
            DecodeLimits::unlimited(),
        ))
        .unwrap_err();

    let FrameCandidateTileBoundaryError::Malformed(
        FrameCandidateTileMalformed::CandidateEnvelopeMismatch { field },
    ) = error
    else {
        panic!("expected candidate/envelope mismatch");
    };
    assert_eq!(field, "payload_source");
}

#[test]
fn derived_boundary_rejects_absent_frame_header_facts_without_guessing() {
    let core = activation_only_frame_header_core();
    let error = FrameCandidateTileFacts::from_frame_core(&core, base_coeff_facts()).unwrap_err();

    let FrameCandidateTileBoundaryError::Unsupported { reason } = error else {
        panic!("expected incomplete frame-header stop");
    };
    assert_eq!(
        reason,
        FrameCandidateTileUnsupportedReason::IncompleteFrameHeader
    );
}

#[test]
fn derived_boundary_rejects_incomplete_tile_group_structure() {
    let bytes = annex_b_tile_group_obu();
    let ctx = context(ThreadCount::from(1usize));
    let envelope = annex_b_envelope(&bytes);
    let facts = base_candidate_facts(false).with_tile_group_structure_start_bits(25);
    let error = derive_tile_payload_plan(
        &ctx,
        &bytes,
        envelope,
        base_position(),
        facts,
        DecodeLimits::unlimited(),
    )
    .unwrap_err();

    let FrameCandidateTileBoundaryError::Malformed(
        FrameCandidateTileMalformed::TileGroupStructureIncomplete,
    ) = error
    else {
        panic!("expected incomplete tile group structure");
    };
}

#[test]
fn derived_boundary_rejects_invalid_locally_parsed_tile_group_structure() {
    let bytes = annex_b_tile_group_obu_with_payload(&[0x00, 0x40, 0x00]);
    let ctx = context(ThreadCount::from(1usize));
    let envelope = annex_b_envelope(&bytes);
    let facts = base_candidate_facts(false).with_tile_group_structure_start_bits(9);
    let error = derive_tile_payload_plan(
        &ctx,
        &bytes,
        envelope,
        base_position(),
        facts,
        DecodeLimits::unlimited(),
    )
    .unwrap_err();

    let FrameCandidateTileBoundaryError::Malformed(
        FrameCandidateTileMalformed::TileGroupStructureInvalid,
    ) = error
    else {
        panic!("expected invalid tile-group structure");
    };
}

#[test]
fn derived_boundary_enforces_tile_count_and_payload_limits() {
    let bytes = annex_b_tile_group_obu();
    let ctx = context(ThreadCount::from(1usize));
    let tile_count_limited =
        DecodeLimits::unlimited().with_max_tile_count(DecodeLimitThreshold::Max(0));
    let error = derive_tile_payload_plan(
        &ctx,
        &bytes,
        annex_b_envelope(&bytes),
        base_position(),
        base_candidate_facts(false),
        tile_count_limited,
    )
    .unwrap_err();

    let FrameCandidateTileBoundaryError::Limit(limit) = error else {
        panic!("expected tile-count limit");
    };
    assert_eq!(limit.name(), DecodeLimitName::MaxTileCount);
    assert_eq!(limit.actual(), Some(1));

    let multi_tile_count_limited =
        DecodeLimits::unlimited().with_max_tile_count(DecodeLimitThreshold::Max(1));
    let error = derive_tile_payload_plan(
        &ctx,
        &bytes,
        annex_b_envelope(&bytes),
        base_position(),
        FrameCandidateTileFacts::new_for_test(
            ObuType::ClosedLoopKey,
            true,
            false,
            2,
            1,
            &[0, 16, 32],
            &[0, 8],
            Some(1),
            0,
            42,
            false,
        ),
        multi_tile_count_limited,
    )
    .unwrap_err();

    let FrameCandidateTileBoundaryError::Limit(limit) = error else {
        panic!("expected multi-tile grid count limit before unsupported tier");
    };
    assert_eq!(limit.name(), DecodeLimitName::MaxTileCount);
    assert_eq!(limit.actual(), Some(2));

    let payload_limited =
        DecodeLimits::unlimited().with_max_tile_payload_bytes(DecodeLimitThreshold::Max(1));
    let error = derive_tile_payload_plan(
        &ctx,
        &bytes,
        annex_b_envelope(&bytes),
        base_position(),
        base_candidate_facts(false),
        payload_limited,
    )
    .unwrap_err();

    let FrameCandidateTileBoundaryError::Boundary(TilePayloadBoundaryError::Limit(limit)) = error
    else {
        panic!("expected tile-payload limit from boundary");
    };
    assert_eq!(limit.name(), DecodeLimitName::MaxTilePayloadBytes);
    assert_eq!(limit.actual(), Some(2));
}

#[test]
fn derived_boundary_rejects_unsupported_position_and_frame_paths() {
    let bytes = annex_b_tile_group_obu();
    let ctx = context(ThreadCount::from(1usize));
    let cases = [
        (
            TileGroupPositionFacts::new(false, true),
            base_candidate_facts(false),
            FrameCandidateTileUnsupportedReason::NonFirstTileGroup,
        ),
        (
            TileGroupPositionFacts::new(true, false),
            base_candidate_facts(false),
            FrameCandidateTileUnsupportedReason::NonLastTileGroup,
        ),
        (
            base_position(),
            FrameCandidateTileFacts::new_for_test(
                ObuType::ClosedLoopKey,
                false,
                false,
                1,
                1,
                &[0, 16],
                &[0, 8],
                None,
                0,
                42,
                false,
            ),
            FrameCandidateTileUnsupportedReason::NonIntraFrame,
        ),
        (
            base_position(),
            FrameCandidateTileFacts::new_for_test(
                ObuType::ClosedLoopKey,
                true,
                true,
                1,
                1,
                &[0, 16],
                &[0, 8],
                None,
                0,
                42,
                false,
            ),
            FrameCandidateTileUnsupportedReason::BridgeFrame,
        ),
        (
            base_position(),
            FrameCandidateTileFacts::new_for_test(
                ObuType::ClosedLoopKey,
                true,
                false,
                2,
                1,
                &[0, 16, 32],
                &[0, 8],
                Some(1),
                0,
                42,
                false,
            ),
            FrameCandidateTileUnsupportedReason::NonSingleTileGroup,
        ),
    ];

    for (position, facts, reason) in cases {
        let error = derive_tile_payload_plan(
            &ctx,
            &bytes,
            annex_b_envelope(&bytes),
            position,
            facts,
            DecodeLimits::unlimited(),
        )
        .unwrap_err();
        let FrameCandidateTileBoundaryError::Unsupported { reason: actual } = error else {
            panic!("expected unsupported derived input");
        };
        assert_eq!(actual, reason);
    }
}

#[test]
fn derived_boundary_is_deterministic_across_decode_context_thread_policies() {
    let bytes = annex_b_tile_group_obu();
    let runtimes = [
        DecodeRuntimeConfig::default(),
        DecodeRuntimeConfig::new(ThreadCount::from(1usize)),
        DecodeRuntimeConfig::new(ThreadCount::from(3usize)),
    ];

    let plans = runtimes.map(|runtime| {
        let ctx = DecodeContext::new(runtime).unwrap();
        derive_tile_payload_plan(
            &ctx,
            &bytes,
            annex_b_envelope(&bytes),
            base_position(),
            base_candidate_facts(false),
            DecodeLimits::unlimited(),
        )
        .unwrap()
    });

    assert_eq!(plans[0], plans[1]);
    assert_eq!(plans[1], plans[2]);
    assert_eq!(plans[0].work_units()[0].tile_bytes(), &[0x80, 0x00]);
}
