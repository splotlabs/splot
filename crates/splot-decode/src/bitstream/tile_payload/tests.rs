// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::cdf::{TileCdfError, TileCdfPolicyInput, TileCdfSelector};
use super::*;
use crate::{DecodeContext, DecodeLimitThreshold, DecodeRuntimeConfig};
use splot_core::headers::tile_group::{TileGroupFraming, parse_tile_group_framing};
use splot_core::symbol::{SymbolDecoder, SymbolDecoderConfig};
use splot_core::types::ObuType;

const MAX: fn(u64) -> DecodeLimitThreshold = DecodeLimitThreshold::Max;
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
    input_with_frame(payload, framing, base_frame(), limits)
}

fn input_with_frame<'a>(
    payload: &'a [u8],
    framing: &'a TileGroupFraming,
    frame: TileFrameFacts,
    limits: DecodeLimits,
) -> TilePayloadBoundaryInput<'a, 'a> {
    TilePayloadBoundaryInput::new(
        payload,
        ByteOffset::new(256),
        framing,
        base_source(),
        base_layer(),
        one_tile_grid(),
        frame,
        limits,
    )
}

fn input_with_grid<'a>(
    payload: &'a [u8],
    framing: &'a TileGroupFraming,
    grid: TileGridFacts<'a>,
    limits: DecodeLimits,
) -> TilePayloadBoundaryInput<'a, 'a> {
    TilePayloadBoundaryInput::new(
        payload,
        ByteOffset::new(256),
        framing,
        base_source(),
        base_layer(),
        grid,
        base_frame(),
        limits,
    )
}

fn unsupported(error: &TilePayloadBoundaryError) -> TilePayloadUnsupported {
    let TilePayloadBoundaryError::Unsupported(unsupported) = error else {
        panic!("expected unsupported tile payload boundary");
    };
    *unsupported
}

fn limit_error(error: &TilePayloadBoundaryError) -> DecodeLimitError {
    let TilePayloadBoundaryError::Limit(limit) = error else {
        panic!("expected tile payload boundary limit");
    };
    *limit
}

#[test]
fn single_tile_payload_yields_deterministic_work_unit_and_unsupported_boundary() {
    let payload = [0x80, 0x00];
    let framing = one_tile_framing(&payload);
    let mut plan =
        plan_tile_payload_boundary(&input(&payload, &framing, DecodeLimits::unlimited())).unwrap();

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
        .read_partition_entry_symbol(selector, &mut symbol)
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
    let input = input_with_frame(&payload, &framing, frame, DecodeLimits::unlimited());
    let plan = plan_tile_payload_boundary(&input).unwrap();

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
    let input = input_with_frame(&payload, &framing, frame, DecodeLimits::unlimited());
    let plan = plan_tile_payload_boundary(&input).unwrap();
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
    let input = input_with_frame(&payload, &framing, frame, DecodeLimits::unlimited());
    let error = plan_tile_payload_boundary(&input).unwrap_err();

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
fn multiple_tiles_are_retained_as_work_units() {
    let payload = [0x00, 0x80, 0x00];
    let framing = parse_tile_group_framing(&payload, 0, 1, 1, false);
    let grid = TileGridFacts::new(2, 1, &[0, 16, 32], &[0, 8]);
    let plan = plan_tile_payload_boundary(&input_with_grid(
        &payload,
        &framing,
        grid,
        DecodeLimits::unlimited(),
    ))
    .unwrap();

    assert_eq!(plan.work_units().len(), 2);
    let first = &plan.work_units()[0];
    assert_eq!(first.tile_num(), 0);
    assert_eq!(first.tile_row(), 0);
    assert_eq!(first.tile_col(), 0);
    assert_eq!(first.mi_row_range(), 0..8);
    assert_eq!(first.mi_col_range(), 0..16);
    assert_eq!(first.tile_bytes(), &[0x80]);
    assert_eq!(
        first.tile_byte_span(),
        ByteSpan::new(ByteOffset::new(257), 1)
    );
    assert!(first.cdf().save_policy().copy_cdf());
    let second = &plan.work_units()[1];
    assert_eq!(second.tile_num(), 1);
    assert_eq!(second.tile_row(), 0);
    assert_eq!(second.tile_col(), 1);
    assert_eq!(second.mi_row_range(), 0..8);
    assert_eq!(second.mi_col_range(), 16..32);
    assert_eq!(second.tile_bytes(), &[0x00]);
    assert_eq!(
        second.tile_byte_span(),
        ByteSpan::new(ByteOffset::new(258), 1)
    );
    assert!(!second.cdf().save_policy().copy_cdf());
}

#[test]
fn inverted_tile_group_range_is_unsupported_without_work_units() {
    let payload = [0x80, 0x00];
    let framing = parse_tile_group_framing(&payload, 2, 1, 1, false);
    assert!(framing.tiles.is_empty());

    let error = plan_tile_payload_boundary(&input(&payload, &framing, DecodeLimits::unlimited()))
        .unwrap_err();
    let unsupported = unsupported(&error);

    assert_eq!(
        unsupported.reason(),
        TilePayloadUnsupportedReason::MissingTileFramingRecords
    );
    assert_eq!(unsupported.tile_num(), None);
}

#[test]
fn single_nonzero_tile_num_is_rejected_by_grid_lookup() {
    let payload = [0x80];
    let framing = parse_tile_group_framing(&payload, 1, 1, 1, false);
    assert_eq!(framing.tiles.len(), 1);
    assert_eq!(framing.tiles[0].tile_num, 1);

    let error = plan_tile_payload_boundary(&input(&payload, &framing, DecodeLimits::unlimited()))
        .unwrap_err();
    let unsupported = unsupported(&error);

    assert_eq!(
        unsupported.reason(),
        TilePayloadUnsupportedReason::InvalidTileGrid
    );
    assert_eq!(unsupported.tile_num(), Some(1));
    assert_eq!(unsupported.byte_offset(), ByteOffset::new(256));
}

#[test]
fn malformed_framing_defect_stops_before_symbol_init() {
    let payload = [];
    let framing = parse_tile_group_framing(&payload, 0, 0, 1, false);
    let error = plan_tile_payload_boundary(&input(&payload, &framing, DecodeLimits::unlimited()))
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
    let limit = limit_error(&error);

    assert_eq!(limit.name(), DecodeLimitName::MaxTilePayloadBytes);
    assert_eq!(limit.op(), Some(DecodeLimitOp::Add));
}

#[test]
fn payload_and_tile_count_limits_are_enforced_first() {
    let payload = [0x80, 0x00];
    let framing = one_tile_framing(&payload);
    let payload_limited =
        DecodeLimits::unlimited().with_max_tile_payload_bytes(DecodeLimitThreshold::Max(1));
    let error =
        plan_tile_payload_boundary(&input(&payload, &framing, payload_limited)).unwrap_err();
    let limit = limit_error(&error);
    assert_eq!(limit.name(), DecodeLimitName::MaxTilePayloadBytes);
    assert_eq!(limit.actual(), Some(2));

    let tile_count_limited =
        DecodeLimits::unlimited().with_max_tile_count(DecodeLimitThreshold::Max(0));
    let error =
        plan_tile_payload_boundary(&input(&payload, &framing, tile_count_limited)).unwrap_err();
    let limit = limit_error(&error);
    assert_eq!(limit.name(), DecodeLimitName::MaxTileCount);
    assert_eq!(limit.actual(), Some(1));

    let exact = DecodeLimits::unlimited()
        .with_max_tile_payload_bytes(MAX(2))
        .with_max_tile_count(MAX(1));
    assert!(plan_tile_payload_boundary(&input(&payload, &framing, exact)).is_ok());
}

#[test]
fn tile_payload_limit_is_per_framed_tile_not_group_payload() {
    let payload = [0x00, 0x80, 0x80];
    let framing = parse_tile_group_framing(&payload, 0, 1, 1, false);
    assert_eq!(framing.tiles.len(), 2);
    assert!(framing.tiles.iter().all(|tile| tile.tile_size == 1));

    let limits = DecodeLimits::unlimited().with_max_tile_payload_bytes(MAX(1));
    let grid = TileGridFacts::new(2, 1, &[0, 16, 32], &[0, 8]);
    let plan =
        plan_tile_payload_boundary(&input_with_grid(&payload, &framing, grid, limits)).unwrap();

    assert_eq!(plan.work_units().len(), 2);
}

#[test]
fn frame_tile_count_limit_uses_grid_not_current_group_len() {
    let payload = [0x80];
    let framing = parse_tile_group_framing(&payload, 1, 1, 1, false);
    assert_eq!(framing.tiles.len(), 1);

    let input = input_with_grid(
        &payload,
        &framing,
        TileGridFacts::new(2, 1, &[0, 16, 32], &[0, 8]),
        DecodeLimits::unlimited().with_max_tile_count(MAX(1)),
    );
    let error = plan_tile_payload_boundary(&input).unwrap_err();
    let limit = limit_error(&error);

    assert_eq!(limit.name(), DecodeLimitName::MaxTileCount);
    assert_eq!(limit.actual(), Some(2));
}

#[test]
fn bridge_frame_keeps_bridge_specific_unsupported_reason() {
    let payload = [0x80];
    let framing = one_tile_framing(&payload);
    let frame = TileFrameFacts::new(
        ObuType::BridgeFrame,
        false,
        true,
        true,
        true,
        TileBruPath::NotUsed,
        0,
        false,
    );
    let input = input_with_frame(&payload, &framing, frame, DecodeLimits::unlimited());
    let error = plan_tile_payload_boundary(&input).unwrap_err();
    let unsupported = unsupported(&error);

    assert_eq!(
        unsupported.reason(),
        TilePayloadUnsupportedReason::BridgeTile
    );
    assert_eq!(unsupported.spec_section(), "5.20.1");
}

#[test]
fn unsupported_minimal_tier_gates_are_structured() {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(splot_parallel::ThreadCount::from(
        1,
    )))
    .unwrap();
    ctx.pool()
        .install(unsupported_minimal_tier_gates_are_structured_inner);
}

fn unsupported_minimal_tier_gates_are_structured_inner() {
    let payload = [0x80];
    let framing = one_tile_framing(&payload);
    let cases = vec![
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
        let input = input_with_frame(&payload, &framing, frame, DecodeLimits::unlimited());
        let error = plan_tile_payload_boundary(&input).unwrap_err();
        let unsupported = unsupported(&error);
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
    let input = input_with_grid(
        &payload,
        &framing,
        TileGridFacts::new(1, 1, &[0], &[0, 8]),
        DecodeLimits::unlimited(),
    );
    let error = plan_tile_payload_boundary(&input).unwrap_err();
    let unsupported = unsupported(&error);
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
        let input = input_with_grid(&payload, &framing, grid, DecodeLimits::unlimited());
        let error = plan_tile_payload_boundary(&input).unwrap_err();
        let unsupported = unsupported(&error);
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
        ctx.plan_tile_payload_boundary(&input(&payload, &framing, DecodeLimits::unlimited()))
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
        .plan_tile_payload_boundary(&input(&payload, &framing, limits))
        .unwrap_err();
    let limit = limit_error(&error);

    assert_eq!(limit.name(), DecodeLimitName::MaxTilePayloadBytes);
    assert_eq!(limit.actual(), Some(2));
}

#[test]
fn arbitrary_small_inputs_do_not_panic() {
    for len in 0..=8usize {
        let payload = vec![0x80; len];
        let framing = one_tile_framing(&payload);
        let _ = plan_tile_payload_boundary(&input(&payload, &framing, DecodeLimits::unlimited()));
    }
}
