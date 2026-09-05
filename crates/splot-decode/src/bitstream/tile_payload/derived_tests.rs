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
use splot_core::headers::sequence::{SequenceHeader, parse_sequence_header};
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::types::ObuType;
use splot_parallel::ThreadCount;

const OBU_CLOSED_LOOP_KEY_HEADER: u8 = 0x10;
const BASE_MI_COL_STARTS: &[u32] = &[0, 16];
const MULTI_TILE_MI_COL_STARTS: &[u32] = &[0, 16, 32];
const BASE_MI_ROW_STARTS: &[u32] = &[0, 8];

struct ParsedFrameHeaderFixture {
    bytes: &'static [u8],
    frame_envelope: ObuEnvelope<'static>,
    sequence: SequenceHeader,
    core: FrameHeaderCore,
}

impl ParsedFrameHeaderFixture {
    fn facts(&self) -> FrameCandidateTileFacts<'_> {
        let tq = self.sequence.transform_quant_entropy.as_ref().unwrap();
        let coeff = FrameCandidateCoeffFacts::from_tq(tq);
        FrameCandidateTileFacts::from_frame_core(&self.core, coeff).unwrap()
    }
}

fn single_thread_context() -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap()
}

fn annex_b_tile_group_obu() -> Vec<u8> {
    annex_b_tile_group_obu_with_payload(&[0x00, 0x80, 0x00])
}

fn annex_b_tile_group_obu_for_type(obu_type: ObuType) -> Vec<u8> {
    let mut bytes = annex_b_tile_group_obu();
    bytes[1] = obu_type.raw() << 2;
    bytes
}

fn annex_b_tile_group_obu_with_payload(payload: &[u8]) -> Vec<u8> {
    let size = u8::try_from(payload.len() + 1).unwrap();
    let mut bytes = vec![size, OBU_CLOSED_LOOP_KEY_HEADER];
    let mut payload = payload.to_vec();
    payload[0] |= 0x80;
    bytes.extend_from_slice(&payload);
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

fn candidate_facts(
    frame_is_intra: bool,
    is_bridge: bool,
    tile_cols: u32,
    mi_col_starts: &'static [u32],
    tile_size_bytes: Option<u32>,
    disable_cdf_update: bool,
) -> FrameCandidateTileFacts<'static> {
    FrameCandidateTileFacts::new_for_test(
        ObuType::ClosedLoopKey,
        frame_is_intra,
        is_bridge,
        tile_cols,
        1,
        mi_col_starts,
        BASE_MI_ROW_STARTS,
        tile_size_bytes,
        0,
        42,
        disable_cdf_update,
    )
}

fn base_candidate_facts(disable_cdf_update: bool) -> FrameCandidateTileFacts<'static> {
    candidate_facts(true, false, 1, BASE_MI_COL_STARTS, None, disable_cdf_update)
}

fn intra_only_tile_group_facts(obu_type: ObuType) -> FrameCandidateTileFacts<'static> {
    FrameCandidateTileFacts::new_for_test(
        obu_type,
        true,
        false,
        1,
        1,
        BASE_MI_COL_STARTS,
        BASE_MI_ROW_STARTS,
        None,
        0,
        42,
        false,
    )
}

fn non_frame_candidate_facts() -> FrameCandidateTileFacts<'static> {
    candidate_facts(false, false, 1, BASE_MI_COL_STARTS, None, false)
}

fn bridge_candidate_facts() -> FrameCandidateTileFacts<'static> {
    candidate_facts(true, true, 1, BASE_MI_COL_STARTS, None, false)
}

fn multi_tile_candidate_facts() -> FrameCandidateTileFacts<'static> {
    candidate_facts(true, false, 2, MULTI_TILE_MI_COL_STARTS, Some(1), false)
}

fn base_position() -> TileGroupPositionFacts {
    TileGroupPositionFacts::new(true, true)
}

fn base_cdf_facts() -> FrameCandidateCdfFacts {
    FrameCandidateCdfFacts::new(false, false)
}

fn base_coeff_facts() -> FrameCandidateCoeffFacts {
    FrameCandidateCoeffFacts::new(false, false, false, false, false, false)
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
    let candidate = stream_plan.frame_candidates_all().next().unwrap();
    let input = FrameCandidateTileBoundaryInput::new(
        &stream_plan,
        candidate,
        bytes,
        envelope,
        position,
        facts,
        base_cdf_facts(),
        limits,
    );
    ctx.pool()
        .install(|| plan_derived_tile_payload_boundary(&input))
}

fn derive_annex_b_tile_payload_plan<'a>(
    ctx: &DecodeContext,
    bytes: &'a [u8],
    facts: FrameCandidateTileFacts<'_>,
    limits: DecodeLimits,
) -> Result<DecodeTilePayloadPlan<'a>, FrameCandidateTileBoundaryError> {
    derive_tile_payload_plan(
        ctx,
        bytes,
        annex_b_envelope(bytes),
        base_position(),
        facts,
        limits,
    )
}

fn parsed_frame_header_fixture() -> ParsedFrameHeaderFixture {
    let bytes: &'static [u8] =
        include_bytes!("../../../../../tests/fixtures/frame-header-core.av2");
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

    ParsedFrameHeaderFixture {
        bytes,
        frame_envelope,
        sequence,
        core,
    }
}

fn activation_only_frame_header_core() -> FrameHeaderCore {
    let data = [0xA0];
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

fn assert_first_work_unit_cdf_update(plan: &DecodeTilePayloadPlan<'_>, expected: CdfUpdateMode) {
    assert_eq!(plan.work_units()[0].symbol().cdf_update_mode(), expected);
    assert_eq!(plan.work_units()[0].cdf().update_mode(), expected);
}

fn assert_malformed_error(
    error: &FrameCandidateTileBoundaryError,
    expected: FrameCandidateTileMalformed,
) {
    let FrameCandidateTileBoundaryError::Malformed(actual) = error else {
        panic!("expected malformed derived input");
    };
    assert_eq!(*actual, expected);
}

fn assert_unsupported_error(
    error: &FrameCandidateTileBoundaryError,
    expected: FrameCandidateTileUnsupportedReason,
) {
    let FrameCandidateTileBoundaryError::Unsupported { reason } = error else {
        panic!("expected unsupported derived input");
    };
    assert_eq!(*reason, expected);
}

fn assert_limit_error(
    error: &FrameCandidateTileBoundaryError,
    expected_name: DecodeLimitName,
    expected_actual: Option<u64>,
) {
    let FrameCandidateTileBoundaryError::Limit(limit) = error else {
        panic!("expected derived limit error");
    };
    assert_eq!(limit.name(), expected_name);
    assert_eq!(limit.actual(), expected_actual);
}

fn assert_boundary_limit_error(
    error: &FrameCandidateTileBoundaryError,
    expected_name: DecodeLimitName,
    expected_actual: Option<u64>,
) {
    let FrameCandidateTileBoundaryError::Boundary(TilePayloadBoundaryError::Limit(limit)) = error
    else {
        panic!("expected boundary limit error");
    };
    assert_eq!(limit.name(), expected_name);
    assert_eq!(limit.actual(), expected_actual);
}

#[test]
fn derived_annex_b_tile_payload_preserves_tile_offsets_and_boundary() {
    let bytes = annex_b_tile_group_obu();
    let ctx = single_thread_context();
    let plan = derive_annex_b_tile_payload_plan(
        &ctx,
        &bytes,
        base_candidate_facts(false),
        DecodeLimits::unlimited(),
    )
    .unwrap();

    assert_eq!(plan.work_units().len(), 1);
    let unit = &plan.work_units()[0];
    assert_eq!(unit.tile_bytes(), &[0x80, 0x00]);
    assert_eq!(unit.tile_byte_span(), ByteSpan::new(ByteOffset::new(3), 2));
    assert_eq!(unit.mi_row_range(), 0..8);
    assert_eq!(unit.mi_col_range(), 0..16);
    assert_first_work_unit_cdf_update(&plan, CdfUpdateMode::Enabled);
    assert!(plan.reaches_last_tile_group());
}

#[test]
fn derived_boundary_admits_intra_only_regular_and_leading_tile_groups() {
    let ctx = single_thread_context();
    for obu_type in [ObuType::RegularTileGroup, ObuType::LeadingTileGroup] {
        let bytes = annex_b_tile_group_obu_for_type(obu_type);
        let plan = derive_annex_b_tile_payload_plan(
            &ctx,
            &bytes,
            intra_only_tile_group_facts(obu_type),
            DecodeLimits::unlimited(),
        )
        .unwrap();
        assert_eq!(plan.work_units().len(), 1);
    }
}

#[test]
fn derived_annex_b_multi_tile_payload_retains_tile_work_units() {
    let bytes = annex_b_tile_group_obu_with_payload(&[0x00, 0x00, 0x00, 0x80, 0x00]);
    let ctx = single_thread_context();
    let plan = derive_annex_b_tile_payload_plan(
        &ctx,
        &bytes,
        multi_tile_candidate_facts(),
        DecodeLimits::unlimited(),
    )
    .unwrap();

    assert_eq!(plan.work_units().len(), 2);
    let first = &plan.work_units()[0];
    assert_eq!(first.tile_num(), 0);
    assert_eq!(first.tile_row(), 0);
    assert_eq!(first.tile_col(), 0);
    assert_eq!(first.mi_row_range(), 0..8);
    assert_eq!(first.mi_col_range(), 0..16);
    assert_eq!(first.tile_byte_span(), ByteSpan::new(ByteOffset::new(5), 1));
    assert_eq!(first.tile_bytes(), &[0x80]);
    assert!(first.cdf().save_policy().copy_cdf);
    let second = &plan.work_units()[1];
    assert_eq!(second.tile_num(), 1);
    assert_eq!(second.tile_row(), 0);
    assert_eq!(second.tile_col(), 1);
    assert_eq!(second.mi_row_range(), 0..8);
    assert_eq!(second.mi_col_range(), 16..32);
    assert_eq!(
        second.tile_byte_span(),
        ByteSpan::new(ByteOffset::new(6), 1)
    );
    assert_eq!(second.tile_bytes(), &[0x00]);
    assert!(!second.cdf().save_policy().copy_cdf);
}

#[test]
fn derived_ivf_tile_payload_preserves_tile_offsets() {
    let frame_payload = annex_b_tile_group_obu();
    let bytes = ivf_with_payload(&frame_payload);
    let ctx = single_thread_context();
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

    assert_eq!(
        plan.work_units()[0].tile_byte_span(),
        ByteSpan::new(ByteOffset::new(47), 2)
    );
    assert_eq!(plan.work_units()[0].tile_bytes(), &[0x80, 0x00]);
}

#[test]
fn derived_boundary_honors_disable_cdf_update_fact() {
    let bytes = annex_b_tile_group_obu();
    let ctx = single_thread_context();
    let plan = derive_annex_b_tile_payload_plan(
        &ctx,
        &bytes,
        base_candidate_facts(true),
        DecodeLimits::unlimited(),
    )
    .unwrap();

    assert_first_work_unit_cdf_update(&plan, CdfUpdateMode::Disabled);
}

#[test]
fn derived_boundary_honors_parser_derived_disable_cdf_update_fact() {
    let fixture = parsed_frame_header_fixture();
    assert_eq!(fixture.core.disable_cdf_update, Some(false));
    let ctx = single_thread_context();
    let plan = derive_tile_payload_plan(
        &ctx,
        fixture.bytes,
        fixture.frame_envelope,
        base_position(),
        fixture.facts(),
        DecodeLimits::unlimited(),
    )
    .unwrap();

    assert_eq!(plan.work_units().len(), 1);
    assert_first_work_unit_cdf_update(&plan, CdfUpdateMode::Enabled);
}

#[test]
fn derived_boundary_threads_parser_coeff_frame_facts() {
    let fixture = parsed_frame_header_fixture();
    let tq = fixture.sequence.transform_quant_entropy.as_ref().unwrap();
    let ctx = single_thread_context();
    let plan = derive_tile_payload_plan(
        &ctx,
        fixture.bytes,
        fixture.frame_envelope,
        base_position(),
        fixture.facts(),
        DecodeLimits::unlimited(),
    )
    .unwrap();
    let coeff_facts = plan.work_units()[0].coeff_frame_facts();
    let lossless = fixture.core.lossless_info.unwrap();
    let tail = fixture.core.intra_tail.unwrap();
    let quant = fixture.core.quantization_params.unwrap();

    assert_eq!(coeff_facts.enable_fsc(), tq.enable_fsc);
    assert_eq!(coeff_facts.enable_intra_ist(), tq.enable_intra_ist);
    assert_eq!(coeff_facts.enable_inter_ist(), tq.enable_inter_ist);
    assert_eq!(
        coeff_facts.enable_chroma_dctonly(),
        tq.enable_chroma_dctonly
    );
    assert_eq!(coeff_facts.enable_cctx(), tq.enable_cctx);
    assert_eq!(coeff_facts.reduced_tx_part_set(), tq.reduced_tx_part_set);
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
    let ctx = single_thread_context();
    let mut envelope = annex_b_envelope(&bytes);
    envelope.size = envelope.size.saturating_add(1);

    let error = derive_tile_payload_plan(
        &ctx,
        &bytes,
        envelope,
        base_position(),
        base_candidate_facts(false),
        DecodeLimits::unlimited(),
    )
    .unwrap_err();

    assert_malformed_error(
        &error,
        FrameCandidateTileMalformed::CandidateEnvelopeMismatch { field: "size" },
    );
}

#[test]
fn derived_boundary_rejects_envelope_payload_from_different_input_buffer() {
    let bytes = annex_b_tile_group_obu();
    let forged_bytes = annex_b_tile_group_obu();
    let ctx = single_thread_context();
    let envelope = annex_b_envelope(&forged_bytes);

    let error = derive_tile_payload_plan(
        &ctx,
        &bytes,
        envelope,
        base_position(),
        base_candidate_facts(false),
        DecodeLimits::unlimited(),
    )
    .unwrap_err();

    assert_malformed_error(
        &error,
        FrameCandidateTileMalformed::CandidateEnvelopeMismatch {
            field: "payload_source",
        },
    );
}

#[test]
fn derived_boundary_rejects_absent_frame_header_facts_without_guessing() {
    let core = activation_only_frame_header_core();
    let error = FrameCandidateTileFacts::from_frame_core(&core, base_coeff_facts()).unwrap_err();

    assert_unsupported_error(
        &error,
        FrameCandidateTileUnsupportedReason::IncompleteFrameHeader,
    );
}

#[test]
fn derived_boundary_rejects_incomplete_tile_group_structure() {
    let bytes = annex_b_tile_group_obu();
    let ctx = single_thread_context();
    let facts = base_candidate_facts(false).with_tile_group_structure_start_bits(25);
    let error = derive_annex_b_tile_payload_plan(&ctx, &bytes, facts, DecodeLimits::unlimited())
        .unwrap_err();

    assert_malformed_error(
        &error,
        FrameCandidateTileMalformed::TileGroupStructureIncomplete,
    );
}

#[test]
fn derived_boundary_rejects_invalid_locally_parsed_tile_group_structure() {
    let bytes = annex_b_tile_group_obu_with_payload(&[0x00, 0x40, 0x00]);
    let ctx = single_thread_context();
    let facts = base_candidate_facts(false).with_tile_group_structure_start_bits(9);
    let error = derive_annex_b_tile_payload_plan(&ctx, &bytes, facts, DecodeLimits::unlimited())
        .unwrap_err();

    assert_malformed_error(
        &error,
        FrameCandidateTileMalformed::TileGroupStructureInvalid,
    );
}

#[test]
fn derived_boundary_enforces_tile_count_and_payload_limits() {
    let bytes = annex_b_tile_group_obu();
    let ctx = single_thread_context();
    let tile_count_limited =
        DecodeLimits::unlimited().with_max_tile_count(DecodeLimitThreshold::Max(0));
    let error = derive_annex_b_tile_payload_plan(
        &ctx,
        &bytes,
        base_candidate_facts(false),
        tile_count_limited,
    )
    .unwrap_err();

    assert_limit_error(&error, DecodeLimitName::MaxTileCount, Some(1));

    let multi_tile_count_limited =
        DecodeLimits::unlimited().with_max_tile_count(DecodeLimitThreshold::Max(1));
    let error = derive_annex_b_tile_payload_plan(
        &ctx,
        &bytes,
        multi_tile_candidate_facts(),
        multi_tile_count_limited,
    )
    .unwrap_err();

    assert_limit_error(&error, DecodeLimitName::MaxTileCount, Some(2));

    let payload_limited =
        DecodeLimits::unlimited().with_max_tile_payload_bytes(DecodeLimitThreshold::Max(1));
    let error = derive_annex_b_tile_payload_plan(
        &ctx,
        &bytes,
        base_candidate_facts(false),
        payload_limited,
    )
    .unwrap_err();

    assert_boundary_limit_error(&error, DecodeLimitName::MaxTilePayloadBytes, Some(2));
}

#[test]
fn derived_boundary_rejects_unsupported_position_and_frame_paths() {
    let bytes = annex_b_tile_group_obu();
    let ctx = single_thread_context();
    for (position, is_first, is_last) in [
        (TileGroupPositionFacts::new(false, true), false, true),
        (TileGroupPositionFacts::new(true, false), true, false),
    ] {
        let error = derive_tile_payload_plan(
            &ctx,
            &bytes,
            annex_b_envelope(&bytes),
            position,
            base_candidate_facts(false),
            DecodeLimits::unlimited(),
        )
        .unwrap_err();
        assert_malformed_error(
            &error,
            FrameCandidateTileMalformed::TileGroupPositionMismatch {
                is_first,
                is_last,
                tg_start: 0,
                tg_end: 0,
                num_tiles: 1,
            },
        );
    }
    let cases = [
        (
            base_position(),
            non_frame_candidate_facts(),
            FrameCandidateTileUnsupportedReason::CandidateNotFrame,
        ),
        (
            base_position(),
            bridge_candidate_facts(),
            FrameCandidateTileUnsupportedReason::CandidateNotFrame,
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
        assert_unsupported_error(&error, reason);
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
