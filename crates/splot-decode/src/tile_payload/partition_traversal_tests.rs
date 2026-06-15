// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use core::ops::Range;

use super::super::cdf::{
    FrameCdfSubset, TileCdfPolicyInput, TileCdfWorkUnitBoundary, tile_cdf_save_policy,
};
use super::super::{SymbolInitBoundary, TileBruPath, TilePayloadSource};
use super::*;
use crate::{DecodeLayerSelection, DecodeLimitError, DecodeLimitThreshold, DecodeObuSourceKind};
use splot_core::span::{ByteOffset, ByteSpan};
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};

const BLOCK_4X4: usize = 0;
const BLOCK_4X8: usize = 1;
const BLOCK_16X16: usize = 6;
const BLOCK_32X32: usize = 9;
const BLOCK_64X64: usize = 12;
const BLOCK_128X128: usize = 15;

static ROW_4X4: [usize; 64] = [BLOCK_4X4; 64];
static ROW_16X16: [usize; 64] = [BLOCK_16X16; 64];
static GRID0: [&[usize]; 2] = [&ROW_4X4, &ROW_16X16];
static GRID1: [&[usize]; 2] = [&ROW_4X4, &ROW_16X16];

fn context() -> TilePartitionContextState<'static> {
    TilePartitionContextState::new([&GRID0, &GRID1], [&ROW_4X4, &ROW_4X4], [&ROW_4X4, &ROW_4X4])
}

fn frame(sb_size: usize) -> TilePartitionFrameFacts {
    TilePartitionFrameFacts::new(
        64,
        64,
        sb_size,
        3,
        true,
        true,
        true,
        false,
        false,
        TilePartitionLoopRestorationState::NoSyntax,
        PartitionFeatureFlags::new(true, true),
        4,
        true,
        TilePartitionBruState::Active,
    )
    .unwrap()
}

fn make_work_unit(
    payload: &'static [u8],
    update_mode: CdfUpdateMode,
) -> DecodeTileWorkUnit<'static> {
    make_work_unit_at(payload, update_mode, 0..64, 0..64)
}

fn make_work_unit_at(
    payload: &'static [u8],
    update_mode: CdfUpdateMode,
    mi_row_range: Range<u32>,
    mi_col_range: Range<u32>,
) -> DecodeTileWorkUnit<'static> {
    DecodeTileWorkUnit {
        source: TilePayloadSource::new(DecodeObuSourceKind::AnnexB, None, 0, ByteOffset::new(0)),
        selected_layer: DecodeLayerSelection::base(),
        tile_num: 0,
        tile_row: 0,
        tile_col: 0,
        mi_row_range,
        mi_col_range,
        tile_bytes: payload,
        tile_byte_span: ByteSpan::new(ByteOffset::new(128), payload.len() as u64),
        tile_size: payload.len() as u64,
        current_q_index_at_entry: 0,
        bru_path: TileBruPath::NotUsed,
        symbol: SymbolInitBoundary {
            consumed_bits: payload.len().saturating_mul(8).min(15) as u64,
            symbol_max_bits: payload.len() as i64 * 8 - 15,
            cdf_update_mode: update_mode,
        },
        cdf: TileCdfWorkUnitBoundary::new(
            update_mode,
            tile_cdf_save_policy(TileCdfPolicyInput::single_tile_default(), 0).unwrap(),
            FrameCdfSubset::from_defaults().tile_copy(),
        ),
    }
}

fn frontier(
    work_unit: &mut DecodeTileWorkUnit<'static>,
    frame: TilePartitionFrameFacts,
    context: TilePartitionContextState<'static>,
) -> Result<TilePartitionTraversalPlan, TilePartitionTraversalError> {
    plan_tile_partition_traversal_frontier(TilePartitionTraversalInput::new(
        work_unit,
        frame,
        context,
        DecodeLimits::DEFAULT,
    ))
}

fn root_call(b_size: usize) -> TilePartitionCall {
    TilePartitionCall::root(0, 0, BlockSize::new(b_size).unwrap(), true)
}

fn child_positions(partition: PartitionType) -> Vec<(usize, usize)> {
    child_positions_for(partition, BLOCK_32X32)
}

fn child_positions_for(partition: PartitionType, b_size: usize) -> Vec<(usize, usize)> {
    let call = root_call(b_size);
    let sub_size = valid_subsize(partition, call.b_size).unwrap();
    child_calls(call, partition, sub_size, frame(b_size), false)
        .unwrap()
        .as_slice()
        .iter()
        .map(|child| (child.r, child.c))
        .collect()
}

#[test]
fn flat_partition_reaches_root_block_frontier() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);

    let plan = frontier(&mut work_unit, frame(BLOCK_32X32), context()).unwrap();

    assert_eq!(
        TILE_PARTITION_TRAVERSAL_MATRIX_ROW,
        "tile-partition-traversal-boundary"
    );
    assert_eq!(
        TILE_PARTITION_TRAVERSAL_FEATURE_ID,
        "DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY"
    );
    assert_eq!(plan.tile_num, 0);
    assert_eq!(plan.steps().len(), 1);
    assert_eq!(plan.steps()[0].decision.partition, PartitionType::None);
    assert_eq!(plan.frontier().r, 0);
    assert_eq!(plan.frontier().c, 0);
    assert_eq!(plan.frontier().b_size.index(), BLOCK_32X32);
    assert!(plan.pending_children().is_empty());
    assert_eq!(plan.symbol_count_after(), 1);
}

#[test]
fn non_none_partition_descends_to_first_child_and_keeps_siblings_pending() {
    let mut work_unit = make_work_unit(&[0xFF, 0x00, 0x80], CdfUpdateMode::Enabled);

    let plan = frontier(&mut work_unit, frame(BLOCK_32X32), context()).unwrap();

    assert!(plan.steps().len() >= 2);
    assert_ne!(plan.steps()[0].decision.partition, PartitionType::None);
    assert_eq!(
        plan.steps().last().unwrap().decision.partition,
        PartitionType::None
    );
    assert_eq!(plan.frontier().r, 0);
    assert_eq!(plan.frontier().c, 0);
    assert!(!plan.pending_children().is_empty());
}

#[test]
fn child_call_geometry_follows_spec_order_for_basic_partitions() {
    assert_eq!(child_positions(PartitionType::Horz), vec![(0, 0), (4, 0)]);
    assert_eq!(child_positions(PartitionType::Vert), vec![(0, 0), (0, 4)]);
    assert_eq!(
        child_positions_for(PartitionType::Split, BLOCK_128X128),
        vec![(0, 0), (0, 16), (16, 0), (16, 16)]
    );
}

#[test]
fn child_call_geometry_follows_spec_order_for_three_way_partitions() {
    assert_eq!(
        child_positions(PartitionType::Horz3),
        vec![(0, 0), (2, 0), (2, 4), (6, 0)]
    );
    assert_eq!(
        child_positions(PartitionType::Vert3),
        vec![(0, 0), (0, 2), (4, 2), (0, 6)]
    );
}

#[test]
fn child_call_geometry_follows_spec_order_for_four_way_partitions() {
    assert_eq!(
        child_positions(PartitionType::Horz4A),
        vec![(0, 0), (1, 0), (3, 0), (7, 0)]
    );
    assert_eq!(
        child_positions(PartitionType::Horz4B),
        vec![(0, 0), (1, 0), (5, 0), (7, 0)]
    );
    assert_eq!(
        child_positions(PartitionType::Vert4A),
        vec![(0, 0), (0, 1), (0, 3), (0, 7)]
    );
    assert_eq!(
        child_positions(PartitionType::Vert4B),
        vec![(0, 0), (0, 1), (0, 5), (0, 7)]
    );
}

#[test]
fn edge_implied_partition_consumes_no_symbol_before_first_child_frontier() {
    let mut work_unit = make_work_unit(&[0xFF, 0xFF], CdfUpdateMode::Enabled);
    let mut facts = frame(BLOCK_32X32);
    facts.mi_cols = 4;

    let plan = frontier(&mut work_unit, facts, context()).unwrap();

    assert_eq!(plan.steps[0].decision.partition, PartitionType::Vert);
    assert_eq!(plan.steps[0].symbol_count_before, 0);
    assert_eq!(plan.steps[0].symbol_count_after, 0);
    assert_eq!(plan.frontier.r, 0);
    assert_eq!(plan.frontier.c, 0);
}

#[test]
fn non_origin_tile_start_availability_uses_tile_bounds() {
    let bounds = TilePartitionBounds {
        mi_row_start: 16,
        mi_row_end: 80,
        mi_col_start: 16,
        mi_col_end: 80,
    };
    let start = TilePartitionCall::root(16, 16, BlockSize::new(BLOCK_32X32).unwrap(), true);
    let top_edge = TilePartitionCall::root(16, 20, BlockSize::new(BLOCK_32X32).unwrap(), true);
    let left_edge = TilePartitionCall::root(20, 16, BlockSize::new(BLOCK_32X32).unwrap(), true);
    let interior = TilePartitionCall::root(20, 20, BlockSize::new(BLOCK_32X32).unwrap(), true);

    assert!(!bounds.avail_u(start));
    assert!(!bounds.avail_l(start));
    assert!(!bounds.avail_u(top_edge));
    assert!(bounds.avail_l(top_edge));
    assert!(bounds.avail_u(left_edge));
    assert!(!bounds.avail_l(left_edge));
    assert!(bounds.avail_u(interior));
    assert!(bounds.avail_l(interior));
}

#[test]
fn non_origin_tile_square_split_does_not_read_neighbors_outside_tile() {
    static EMPTY_GRID: [&[usize]; 0] = [];
    static LONG_ROW: [usize; 256] = [BLOCK_4X4; 256];
    let sparse_context = TilePartitionContextState::new(
        [&EMPTY_GRID, &EMPTY_GRID],
        [&LONG_ROW, &LONG_ROW],
        [&LONG_ROW, &LONG_ROW],
    );
    let work_unit = make_work_unit_at(
        &[0xFF, 0x00, 0x80],
        CdfUpdateMode::Enabled,
        16..160,
        16..160,
    );
    let mut facts = frame(BLOCK_128X128);
    facts.mi_rows = 256;
    facts.mi_cols = 256;
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled);
    let mut symbols = SymbolDecoder::with_base_and_config(
        work_unit.tile_bytes(),
        work_unit.tile_byte_span().start,
        config,
    )
    .unwrap();

    let decision = read_frontier_partition_decision(
        TilePartitionCall::root(16, 16, BlockSize::new(BLOCK_128X128).unwrap(), true),
        facts,
        TilePartitionBounds::from_work_unit(&work_unit),
        sparse_context,
        &mut cdfs,
        &mut symbols,
    )
    .unwrap();

    assert!(
        decision.trace.do_square_split.is_some(),
        "expected do_square_split to be read, got {decision:?}"
    );
}

#[test]
fn chroma_offset_update_preserves_decode_block_has_chroma() {
    let mut work_unit = make_work_unit(&[0x80], CdfUpdateMode::Enabled);

    let plan = frontier(&mut work_unit, frame(BLOCK_4X4), context()).unwrap();

    assert_eq!(plan.frontier.b_size.index(), BLOCK_4X4);
    assert!(plan.frontier.chroma_offset);
    assert!(plan.frontier.has_chroma);
}

#[test]
fn split_child_sdp_gate_rejects_nested_64x64_calls() {
    let root = root_call(BLOCK_128X128);
    let sub_size = valid_subsize(PartitionType::Split, root.b_size).unwrap();
    let children = child_calls(
        root,
        PartitionType::Split,
        sub_size,
        frame(BLOCK_128X128),
        false,
    )
    .unwrap();
    let mut facts = frame(BLOCK_128X128);
    facts.enable_sdp = true;

    let err = ensure_supported_call(facts, children.as_slice()[0]).unwrap_err();

    assert_eq!(children.as_slice()[0].b_size.index(), BLOCK_64X64);
    assert!(matches!(
        err,
        TilePartitionTraversalError::Unsupported(TilePartitionTraversalUnsupported::Sdp)
    ));
}

#[test]
fn failed_context_read_does_not_commit_cdf_mutation() {
    static EMPTY: [usize; 0] = [];
    static EMPTY_GRID: [&[usize]; 0] = [];
    let bad_context = TilePartitionContextState::new(
        [&EMPTY_GRID, &EMPTY_GRID],
        [&EMPTY, &EMPTY],
        [&EMPTY, &EMPTY],
    );
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let before = work_unit.cdf().tile_cdfs().clone();

    let err = frontier(&mut work_unit, frame(BLOCK_32X32), bad_context).unwrap_err();

    assert!(matches!(
        err,
        TilePartitionTraversalError::Decision(PartitionDecisionError::Cdf(
            TileCdfError::PartitionNeighborOutOfRange { .. }
        ))
    ));
    assert_eq!(work_unit.cdf().tile_cdfs(), &before);
}

#[test]
fn read_lr_gate_precedes_partition_symbol_reads() {
    let mut work_unit = make_work_unit(&[], CdfUpdateMode::Enabled);
    let before = work_unit.cdf().tile_cdfs().clone();
    let mut facts = frame(BLOCK_32X32);
    facts.loop_restoration = TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;

    let err = frontier(&mut work_unit, facts, context()).unwrap_err();

    assert!(matches!(
        err,
        TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ReadLoopRestoration
        )
    ));
    assert_eq!(work_unit.cdf().tile_cdfs(), &before);
}

#[test]
fn disabled_cdf_update_preserves_rows_while_advancing_symbols() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Disabled);
    let before = work_unit.cdf().tile_cdfs().clone();

    let plan = frontier(&mut work_unit, frame(BLOCK_32X32), context()).unwrap();

    assert_eq!(plan.symbol_count_after(), 1);
    assert_eq!(work_unit.cdf().tile_cdfs(), &before);
}

#[test]
fn unsupported_gates_are_explicit() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut sdp = frame(BLOCK_64X64);
    sdp.enable_sdp = true;
    let err = frontier(&mut work_unit, sdp, context()).unwrap_err();
    assert!(matches!(
        err,
        TilePartitionTraversalError::Unsupported(TilePartitionTraversalUnsupported::Sdp)
    ));

    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut extended_sdp = frame(BLOCK_32X32);
    extended_sdp.enable_extended_sdp = true;
    let err = frontier(&mut work_unit, extended_sdp, context()).unwrap_err();
    assert!(matches!(
        err,
        TilePartitionTraversalError::Unsupported(TilePartitionTraversalUnsupported::ExtendedSdp)
    ));

    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut read_lr = frame(BLOCK_32X32);
    read_lr.loop_restoration = TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
    let err = frontier(&mut work_unit, read_lr, context()).unwrap_err();
    assert!(matches!(
        err,
        TilePartitionTraversalError::Unsupported(
            TilePartitionTraversalUnsupported::ReadLoopRestoration
        )
    ));

    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut inter = frame(BLOCK_32X32);
    inter.frame_is_intra = false;
    let err = frontier(&mut work_unit, inter, context()).unwrap_err();
    assert!(matches!(
        err,
        TilePartitionTraversalError::Unsupported(TilePartitionTraversalUnsupported::NonIntra)
    ));

    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut bru = frame(BLOCK_32X32);
    bru.bru_state = TilePartitionBruState::Unsupported;
    let err = frontier(&mut work_unit, bru, context()).unwrap_err();
    assert!(matches!(
        err,
        TilePartitionTraversalError::Unsupported(TilePartitionTraversalUnsupported::BruOrBridge)
    ));
}

#[test]
fn arithmetic_and_invalid_subsize_errors_are_typed() {
    let add = checked_add("r", usize::MAX, 1).unwrap_err();
    assert!(matches!(
        add,
        TilePartitionTraversalError::CoordinateOverflow {
            coordinate: "r",
            base: usize::MAX,
            offset: 1
        }
    ));

    let scaled = checked_scaled_add("c", 0, usize::MAX, 2).unwrap_err();
    assert!(matches!(
        scaled,
        TilePartitionTraversalError::CoordinateOffsetOverflow {
            coordinate: "c",
            left: usize::MAX,
            right: 2
        }
    ));

    let invalid =
        valid_subsize(PartitionType::Vert, BlockSize::new(BLOCK_4X8).unwrap()).unwrap_err();
    assert!(matches!(
        invalid,
        TilePartitionTraversalError::InvalidPartitionSubsize {
            partition: PartitionType::Vert,
            b_size: BLOCK_4X8,
        }
    ));
}

#[test]
fn max_tile_partition_steps_limit_bounds_frontier_steps() {
    let mut work_unit = make_work_unit(&[0xFF, 0x00, 0x80], CdfUpdateMode::Enabled);
    let err = plan_tile_partition_traversal_frontier(TilePartitionTraversalInput::new(
        &mut work_unit,
        frame(BLOCK_32X32),
        context(),
        DecodeLimits::unlimited().with_max_tile_partition_steps(DecodeLimitThreshold::Max(1)),
    ))
    .unwrap_err();

    assert!(matches!(
        &err,
        TilePartitionTraversalError::Limit(DecodeLimitError::LimitExceeded { .. })
    ));
    if let TilePartitionTraversalError::Limit(DecodeLimitError::LimitExceeded { check }) = err {
        assert_eq!(check.name(), DecodeLimitName::MaxTilePartitionSteps);
        assert_eq!(check.actual(), 2);
    }
}

#[test]
fn max_tile_count_limit_does_not_bound_frontier_steps() {
    let mut work_unit = make_work_unit(&[0xFF, 0x00, 0x80], CdfUpdateMode::Enabled);
    let plan = plan_tile_partition_traversal_frontier(TilePartitionTraversalInput::new(
        &mut work_unit,
        frame(BLOCK_32X32),
        context(),
        DecodeLimits::unlimited()
            .with_max_tile_count(DecodeLimitThreshold::Max(1))
            .with_max_tile_partition_steps(DecodeLimitThreshold::Max(8)),
    ))
    .unwrap();

    assert!(plan.steps().len() > 1);
}
