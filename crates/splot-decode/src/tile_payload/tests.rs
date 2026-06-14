// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::cdf::{TileCdfError, TileCdfPolicyInput, TileCdfSelector};
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
use splot_core::headers::tile_group::{TileGroupFraming, parse_tile_group_framing};
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::symbol::{SymbolDecoder, SymbolDecoderConfig};
use splot_core::types::ObuType;
use splot_parallel::ThreadCount;

const MAX: fn(u64) -> DecodeLimitThreshold = DecodeLimitThreshold::Max;
const OBU_CLOSED_LOOP_KEY_HEADER: u8 = 0x10;

fn base_source() -> TilePayloadSource {
    TilePayloadSource::new(DecodeObuSourceKind::AnnexB, None, 7, ByteOffset::new(100))
}

fn base_layer() -> DecodeLayerSelection {
    DecodeLayerSelection::base()
}

fn one_tile_grid() -> TileGridFacts<'static> {
    TileGridFacts::new(1, 1, &[0, 16], &[0, 8])
}

fn base_frame() -> TileFrameFacts {
    TileFrameFacts::new(
        ObuType::ClosedLoopKey,
        true,
        true,
        true,
        false,
        TileBruPath::NotUsed,
        42,
        false,
    )
}

fn one_tile_framing(payload: &[u8]) -> TileGroupFraming {
    parse_tile_group_framing(payload, 0, 0, 1, false)
}

fn input<'a>(
    payload: &'a [u8],
    framing: &'a TileGroupFraming,
    limits: DecodeLimits,
) -> TilePayloadBoundaryInput<'a, 'a> {
    TilePayloadBoundaryInput::new(
        payload,
        ByteOffset::new(256),
        framing,
        base_source(),
        base_layer(),
        one_tile_grid(),
        base_frame(),
        limits,
    )
}

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
fn single_tile_payload_yields_deterministic_work_unit_and_unsupported_boundary() {
    let payload = [0x80, 0x00];
    let framing = one_tile_framing(&payload);
    let mut plan =
        plan_tile_payload_boundary(input(&payload, &framing, DecodeLimits::unlimited())).unwrap();

    assert_eq!(plan.source(), base_source());
    assert_eq!(plan.source().source_kind(), DecodeObuSourceKind::AnnexB);
    assert_eq!(plan.source().ivf_frame(), None);
    assert_eq!(plan.source().obu_index(), 7);
    assert_eq!(plan.source().obu_offset(), ByteOffset::new(100));
    assert_eq!(plan.selected_layer(), DecodeLayerSelection::base());
    assert_eq!(plan.work_units().len(), 1);
    let unit = &plan.work_units()[0];
    assert_eq!(unit.source(), base_source());
    assert_eq!(unit.selected_layer(), DecodeLayerSelection::base());
    assert_eq!(unit.tile_num(), 0);
    assert_eq!(unit.tile_row(), 0);
    assert_eq!(unit.tile_col(), 0);
    assert_eq!(unit.mi_row_range(), 0..8);
    assert_eq!(unit.mi_col_range(), 0..16);
    assert_eq!(unit.tile_bytes(), &payload);
    assert_eq!(
        unit.tile_byte_span(),
        ByteSpan::new(ByteOffset::new(256), 2)
    );
    assert_eq!(unit.tile_size(), 2);
    assert_eq!(unit.current_q_index_at_entry(), 42);
    assert_eq!(unit.symbol().consumed_bits(), 15);
    assert_eq!(unit.symbol().symbol_max_bits(), 1);
    assert_eq!(unit.symbol().cdf_update_mode(), CdfUpdateMode::Enabled);
    assert_eq!(unit.cdf().update_mode(), CdfUpdateMode::Enabled);
    assert!(unit.cdf().save_policy().copy_cdf());
    assert!(!unit.cdf().save_policy().avg_cdf());
    let selector = TileCdfSelector::DoSplit {
        plane_start: 0,
        ctx: 0,
    };
    assert_eq!(
        unit.cdf().tile_cdfs().row(selector).unwrap(),
        splot_core::tables::cdf::DEFAULT_DO_SPLIT_CDF[0][0].as_slice()
    );

    let cdf_before = unit.cdf().tile_cdfs().row(selector).unwrap().to_vec();
    let mut symbol = SymbolDecoder::with_base_and_config(
        &payload,
        ByteOffset::new(256),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap();
    plan.work_units_mut()[0]
        .cdf_mut()
        .tile_cdfs_mut()
        .with_row_mut(selector, |row| symbol.read_symbol(row))
        .unwrap()
        .unwrap();
    assert_ne!(
        plan.work_units()[0]
            .cdf()
            .tile_cdfs()
            .row(selector)
            .unwrap(),
        cdf_before.as_slice()
    );

    let unsupported = plan.unsupported();
    assert_eq!(unsupported.rule_id(), UNSUPPORTED_FEATURE_RULE_ID);
    assert_eq!(unsupported.matrix_row(), TILE_PAYLOAD_DECODE_MATRIX_ROW);
    assert_eq!(unsupported.feature_id(), TILE_PAYLOAD_DECODE_FEATURE_ID);
    assert_eq!(unsupported.spec_section(), "5.20.2.1");
    assert_eq!(
        unsupported.reason(),
        TilePayloadUnsupportedReason::DecodeTileSyntax
    );
    assert_eq!(unsupported.tile_num(), Some(0));
    assert_eq!(unsupported.byte_offset(), ByteOffset::new(256));
    assert!(unsupported.message().contains("decode_tile()"));
    assert!(plan.frame_end().reaches_last_tile_group());
    assert!(plan.frame_end().frame_end_update_cdf_deferred());
    assert!(plan.frame_end().decode_frame_wrapup_deferred());
}

#[test]
fn cdf_update_disable_is_recorded_without_symbol_finish() {
    let payload = [0x80];
    let framing = one_tile_framing(&payload);
    let mut frame = base_frame();
    frame.disable_cdf_update = true;
    let input = TilePayloadBoundaryInput::new(
        &payload,
        ByteOffset::new(8),
        &framing,
        base_source(),
        base_layer(),
        one_tile_grid(),
        frame,
        DecodeLimits::unlimited(),
    );
    let plan = plan_tile_payload_boundary(input).unwrap();

    assert_eq!(
        plan.work_units()[0].symbol().cdf_update_mode(),
        CdfUpdateMode::Disabled
    );
    assert_eq!(
        plan.work_units()[0].cdf().update_mode(),
        CdfUpdateMode::Disabled
    );
    assert_eq!(plan.work_units()[0].symbol().symbol_max_bits(), -7);
    assert_eq!(
        plan.unsupported().reason(),
        TilePayloadUnsupportedReason::DecodeTileSyntax
    );
}

#[test]
fn cdf_policy_tile_dimensions_are_derived_from_planned_grid() {
    let payload = [0x80, 0x00];
    let framing = one_tile_framing(&payload);
    let frame = base_frame().with_cdf_policy(TileCdfPolicyInput::new(16, 1, true, true, 0));
    let input = TilePayloadBoundaryInput::new(
        &payload,
        ByteOffset::new(16),
        &framing,
        base_source(),
        base_layer(),
        one_tile_grid(),
        frame,
        DecodeLimits::unlimited(),
    );
    let plan = plan_tile_payload_boundary(input).unwrap();
    let save_policy = plan.work_units()[0].cdf().save_policy();

    assert_eq!(save_policy.num_log2(), 0);
    assert!(save_policy.avg_cdf());
    assert!(!save_policy.copy_cdf());
}

#[test]
fn cdf_context_update_tile_id_is_validated_against_planned_grid() {
    let payload = [0x80, 0x00];
    let framing = one_tile_framing(&payload);
    let frame = base_frame().with_cdf_policy(TileCdfPolicyInput::new(16, 1, false, false, 2));
    let input = TilePayloadBoundaryInput::new(
        &payload,
        ByteOffset::new(16),
        &framing,
        base_source(),
        base_layer(),
        one_tile_grid(),
        frame,
        DecodeLimits::unlimited(),
    );
    let error = plan_tile_payload_boundary(input).unwrap_err();

    let TilePayloadBoundaryError::Cdf(TileCdfError::ContextUpdateTileOutOfRange {
        context_update_tile_id,
        tile_count,
    }) = error
    else {
        panic!("expected cdf context-update tile error");
    };
    assert_eq!(context_update_tile_id, 2);
    assert_eq!(tile_count, 1);
}

#[test]
fn multiple_tiles_are_unsupported_before_work_units_are_retained() {
    let payload = [0x00, 0x80, 0x00];
    let framing = parse_tile_group_framing(&payload, 0, 1, 1, false);
    let error = plan_tile_payload_boundary(input(&payload, &framing, DecodeLimits::unlimited()))
        .unwrap_err();

    let TilePayloadBoundaryError::Unsupported(unsupported) = error else {
        panic!("expected unsupported multiple tiles");
    };
    assert_eq!(
        unsupported.reason(),
        TilePayloadUnsupportedReason::NonSingleTile
    );
    assert_eq!(unsupported.tile_num(), None);
}

#[test]
fn inverted_tile_group_range_is_unsupported_without_work_units() {
    let payload = [0x80, 0x00];
    let framing = parse_tile_group_framing(&payload, 2, 1, 1, false);
    assert!(framing.tiles.is_empty());

    let error = plan_tile_payload_boundary(input(&payload, &framing, DecodeLimits::unlimited()))
        .unwrap_err();

    let TilePayloadBoundaryError::Unsupported(unsupported) = error else {
        panic!("expected unsupported empty tile group");
    };
    assert_eq!(
        unsupported.reason(),
        TilePayloadUnsupportedReason::NonSingleTile
    );
    assert_eq!(unsupported.tile_num(), None);
}

#[test]
fn single_nonzero_tile_num_is_unsupported_before_grid_lookup() {
    let payload = [0x80];
    let framing = parse_tile_group_framing(&payload, 1, 1, 1, false);
    assert_eq!(framing.tiles.len(), 1);
    assert_eq!(framing.tiles[0].tile_num, 1);

    let error = plan_tile_payload_boundary(input(&payload, &framing, DecodeLimits::unlimited()))
        .unwrap_err();

    let TilePayloadBoundaryError::Unsupported(unsupported) = error else {
        panic!("expected unsupported nonzero tile number");
    };
    assert_eq!(
        unsupported.reason(),
        TilePayloadUnsupportedReason::MultipleTiles
    );
    assert_eq!(unsupported.tile_num(), Some(1));
    assert_eq!(unsupported.byte_offset(), ByteOffset::new(256));
}

#[test]
fn malformed_framing_defect_stops_before_symbol_init() {
    let payload = [];
    let framing = parse_tile_group_framing(&payload, 0, 0, 1, false);
    let error = plan_tile_payload_boundary(input(&payload, &framing, DecodeLimits::unlimited()))
        .unwrap_err();

    let TilePayloadBoundaryError::Malformed(TilePayloadMalformed::FramingDefect(defect)) = error
    else {
        panic!("expected framing defect");
    };
    assert_eq!(defect.label(), "zero-size-tile");
}

#[test]
fn tile_range_out_of_bounds_is_malformed_not_panic() {
    let payload = [0x80];
    let error = tile_slice(&payload, 0, 0, 2).unwrap_err();

    let TilePayloadBoundaryError::Malformed(TilePayloadMalformed::TileRangeOutOfBounds {
        tile_num,
        tile_data_offset,
        tile_size,
        payload_len,
    }) = error
    else {
        panic!("expected range error");
    };
    assert_eq!(tile_num, 0);
    assert_eq!(tile_data_offset, 0);
    assert_eq!(tile_size, 2);
    assert_eq!(payload_len, 1);
}

#[test]
fn constructed_huge_tile_range_is_rejected_before_slice_indexing() {
    let payload = [0x80];
    let error = tile_slice(&payload, 0, u64::MAX, 1).unwrap_err();

    let TilePayloadBoundaryError::Limit(limit) = error else {
        panic!("expected arithmetic overflow limit");
    };
    assert_eq!(limit.name(), DecodeLimitName::MaxTilePayloadBytes);
    assert_eq!(limit.op(), Some(DecodeLimitOp::Add));
}

#[test]
fn payload_and_tile_count_limits_are_enforced_first() {
    let payload = [0x80, 0x00];
    let framing = one_tile_framing(&payload);
    let payload_limited =
        DecodeLimits::unlimited().with_max_tile_payload_bytes(DecodeLimitThreshold::Max(1));
    let error = plan_tile_payload_boundary(input(&payload, &framing, payload_limited)).unwrap_err();
    let TilePayloadBoundaryError::Limit(limit) = error else {
        panic!("expected payload limit");
    };
    assert_eq!(limit.name(), DecodeLimitName::MaxTilePayloadBytes);
    assert_eq!(limit.actual(), Some(2));

    let tile_count_limited =
        DecodeLimits::unlimited().with_max_tile_count(DecodeLimitThreshold::Max(0));
    let error =
        plan_tile_payload_boundary(input(&payload, &framing, tile_count_limited)).unwrap_err();
    let TilePayloadBoundaryError::Limit(limit) = error else {
        panic!("expected tile count limit");
    };
    assert_eq!(limit.name(), DecodeLimitName::MaxTileCount);
    assert_eq!(limit.actual(), Some(1));

    let exact = DecodeLimits::unlimited()
        .with_max_tile_payload_bytes(MAX(2))
        .with_max_tile_count(MAX(1));
    assert!(plan_tile_payload_boundary(input(&payload, &framing, exact)).is_ok());
}

#[test]
fn tile_payload_limit_is_per_framed_tile_not_group_payload() {
    let payload = [0x00, 0x80, 0x80];
    let framing = parse_tile_group_framing(&payload, 0, 1, 1, false);
    assert_eq!(framing.tiles.len(), 2);
    assert!(framing.tiles.iter().all(|tile| tile.tile_size == 1));

    let limits = DecodeLimits::unlimited().with_max_tile_payload_bytes(MAX(1));
    let error = plan_tile_payload_boundary(input(&payload, &framing, limits)).unwrap_err();

    let TilePayloadBoundaryError::Unsupported(unsupported) = error else {
        panic!("expected unsupported non-single-tile group");
    };
    assert_eq!(
        unsupported.reason(),
        TilePayloadUnsupportedReason::NonSingleTile
    );
}

#[test]
fn frame_tile_count_limit_uses_grid_not_current_group_len() {
    let payload = [0x80];
    let framing = parse_tile_group_framing(&payload, 1, 1, 1, false);
    assert_eq!(framing.tiles.len(), 1);

    let input = TilePayloadBoundaryInput::new(
        &payload,
        ByteOffset::new(0),
        &framing,
        base_source(),
        base_layer(),
        TileGridFacts::new(2, 1, &[0, 16, 32], &[0, 8]),
        base_frame(),
        DecodeLimits::unlimited().with_max_tile_count(MAX(1)),
    );
    let error = plan_tile_payload_boundary(input).unwrap_err();

    let TilePayloadBoundaryError::Limit(limit) = error else {
        panic!("expected frame tile count limit");
    };
    assert_eq!(limit.name(), DecodeLimitName::MaxTileCount);
    assert_eq!(limit.actual(), Some(2));
}

#[test]
fn bridge_frame_keeps_bridge_specific_unsupported_reason() {
    let payload = [0x80];
    let framing = one_tile_framing(&payload);
    let input = TilePayloadBoundaryInput::new(
        &payload,
        ByteOffset::new(0),
        &framing,
        base_source(),
        base_layer(),
        one_tile_grid(),
        TileFrameFacts::new(
            ObuType::BridgeFrame,
            false,
            true,
            true,
            true,
            TileBruPath::NotUsed,
            0,
            false,
        ),
        DecodeLimits::unlimited(),
    );
    let error = plan_tile_payload_boundary(input).unwrap_err();

    let TilePayloadBoundaryError::Unsupported(unsupported) = error else {
        panic!("expected bridge unsupported reason");
    };
    assert_eq!(
        unsupported.reason(),
        TilePayloadUnsupportedReason::BridgeTile
    );
    assert_eq!(unsupported.spec_section(), "5.20.1");
}

#[test]
fn unsupported_minimal_tier_gates_are_structured() {
    let payload = [0x80];
    let framing = one_tile_framing(&payload);
    let cases = [
        (
            TileFrameFacts::new(
                ObuType::OpenLoopKey,
                true,
                true,
                true,
                false,
                TileBruPath::NotUsed,
                0,
                false,
            ),
            TilePayloadUnsupportedReason::NonClosedLoopKey,
            "7.1",
        ),
        (
            TileFrameFacts::new(
                ObuType::ClosedLoopKey,
                false,
                true,
                true,
                false,
                TileBruPath::NotUsed,
                0,
                false,
            ),
            TilePayloadUnsupportedReason::NonIntraFrame,
            "7.1",
        ),
        (
            TileFrameFacts::new(
                ObuType::ClosedLoopKey,
                true,
                false,
                true,
                false,
                TileBruPath::NotUsed,
                0,
                false,
            ),
            TilePayloadUnsupportedReason::MissingCompleteIntraFirstTileGroup,
            "5.20.1",
        ),
        (
            TileFrameFacts::new(
                ObuType::ClosedLoopKey,
                true,
                true,
                false,
                false,
                TileBruPath::NotUsed,
                0,
                false,
            ),
            TilePayloadUnsupportedReason::MultipleTileGroups,
            "5.20.1",
        ),
        (
            TileFrameFacts::new(
                ObuType::ClosedLoopKey,
                true,
                true,
                true,
                true,
                TileBruPath::NotUsed,
                0,
                false,
            ),
            TilePayloadUnsupportedReason::BridgeTile,
            "5.20.1",
        ),
        (
            TileFrameFacts::new(
                ObuType::ClosedLoopKey,
                true,
                true,
                true,
                false,
                TileBruPath::Active,
                0,
                false,
            ),
            TilePayloadUnsupportedReason::BruTileActivity,
            "5.20.1",
        ),
        (
            TileFrameFacts::new(
                ObuType::ClosedLoopKey,
                true,
                true,
                true,
                false,
                TileBruPath::Inactive,
                0,
                false,
            ),
            TilePayloadUnsupportedReason::BruTileActivity,
            "5.20.1",
        ),
    ];

    for (frame, reason, spec_section) in cases {
        let input = TilePayloadBoundaryInput::new(
            &payload,
            ByteOffset::new(40),
            &framing,
            base_source(),
            base_layer(),
            one_tile_grid(),
            frame,
            DecodeLimits::unlimited(),
        );
        let error = plan_tile_payload_boundary(input).unwrap_err();
        let TilePayloadBoundaryError::Unsupported(unsupported) = error else {
            panic!("expected unsupported gate");
        };
        assert_eq!(unsupported.reason(), reason);
        assert_eq!(unsupported.rule_id(), "decode/unsupported-feature");
        assert_eq!(unsupported.matrix_row(), "tile-payload-decode");
        assert_eq!(unsupported.feature_id(), "DECODE-TILE-PAYLOAD-BOUNDARY");
        assert_eq!(unsupported.spec_section(), spec_section);
    }
}

#[test]
fn invalid_grid_and_offset_overflow_are_structured() {
    let payload = [0x80];
    let framing = one_tile_framing(&payload);
    let input = TilePayloadBoundaryInput::new(
        &payload,
        ByteOffset::new(0),
        &framing,
        base_source(),
        base_layer(),
        TileGridFacts::new(1, 1, &[0], &[0, 8]),
        base_frame(),
        DecodeLimits::unlimited(),
    );
    let error = plan_tile_payload_boundary(input).unwrap_err();
    let TilePayloadBoundaryError::Unsupported(unsupported) = error else {
        panic!("expected invalid grid unsupported");
    };
    assert_eq!(
        unsupported.reason(),
        TilePayloadUnsupportedReason::InvalidTileGrid
    );

    let overflow = checked_byte_offset(
        ByteOffset::new(u64::MAX),
        1,
        DecodeLimitName::MaxTilePayloadBytes,
    )
    .unwrap_err();
    assert_eq!(overflow.name(), DecodeLimitName::MaxTilePayloadBytes);
    assert_eq!(overflow.op(), Some(DecodeLimitOp::Add));

    let span_overflow = checked_byte_span(
        ByteOffset::new(u64::MAX),
        1,
        DecodeLimitName::MaxTilePayloadBytes,
    )
    .unwrap_err();
    assert_eq!(span_overflow.name(), DecodeLimitName::MaxTilePayloadBytes);
    assert_eq!(span_overflow.op(), Some(DecodeLimitOp::Add));
}

#[test]
fn non_increasing_mi_grid_ranges_are_invalid() {
    let payload = [0x80];
    let framing = one_tile_framing(&payload);
    let grids = [
        TileGridFacts::new(1, 1, &[0, 16], &[8, 8]),
        TileGridFacts::new(1, 1, &[0, 16], &[8, 0]),
        TileGridFacts::new(1, 1, &[4, 4], &[0, 8]),
        TileGridFacts::new(1, 1, &[16, 0], &[0, 8]),
    ];

    for grid in grids {
        let input = TilePayloadBoundaryInput::new(
            &payload,
            ByteOffset::new(0),
            &framing,
            base_source(),
            base_layer(),
            grid,
            base_frame(),
            DecodeLimits::unlimited(),
        );

        let error = plan_tile_payload_boundary(input).unwrap_err();
        let TilePayloadBoundaryError::Unsupported(unsupported) = error else {
            panic!("expected invalid grid unsupported");
        };
        assert_eq!(
            unsupported.reason(),
            TilePayloadUnsupportedReason::InvalidTileGrid
        );
    }
}

#[test]
fn boundary_is_deterministic_through_decode_context_worker_pool() {
    let payload = [0x80, 0x00];
    let framing = one_tile_framing(&payload);
    let runtimes = [
        DecodeRuntimeConfig::default(),
        DecodeRuntimeConfig::new(splot_parallel::ThreadCount::from(1)),
        DecodeRuntimeConfig::new(splot_parallel::ThreadCount::from(3)),
    ];

    let plans = runtimes.map(|runtime| {
        let ctx = DecodeContext::new(runtime).unwrap();
        ctx.plan_tile_payload_boundary(input(&payload, &framing, DecodeLimits::unlimited()))
            .unwrap()
    });

    assert_eq!(plans[0], plans[1]);
    assert_eq!(plans[1], plans[2]);
    assert_eq!(plans[0].work_units()[0].tile_bytes(), &payload);
}

#[test]
fn decode_context_tile_payload_handoff_preserves_limit_errors() {
    let payload = [0x80, 0x00];
    let framing = one_tile_framing(&payload);
    let ctx = DecodeContext::new(DecodeRuntimeConfig::default()).unwrap();
    let limits = DecodeLimits::unlimited().with_max_tile_payload_bytes(MAX(1));

    let error = ctx
        .plan_tile_payload_boundary(input(&payload, &framing, limits))
        .unwrap_err();

    let TilePayloadBoundaryError::Limit(limit) = error else {
        panic!("expected payload limit");
    };
    assert_eq!(limit.name(), DecodeLimitName::MaxTilePayloadBytes);
    assert_eq!(limit.actual(), Some(2));
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
    let facts = FrameCandidateTileFacts::from_frame_core(&core).unwrap();

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
    let error = FrameCandidateTileFacts::from_frame_core(&core).unwrap_err();

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

#[test]
fn arbitrary_small_inputs_do_not_panic() {
    for len in 0..=8usize {
        let payload = vec![0x80; len];
        let framing = one_tile_framing(&payload);
        let _ = plan_tile_payload_boundary(input(&payload, &framing, DecodeLimits::unlimited()));
    }
}
