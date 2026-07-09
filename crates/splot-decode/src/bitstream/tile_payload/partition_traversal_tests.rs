// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use core::ops::Range;

use super::super::cdf::{
    FrameCdfSubset, TileCdfPolicyInput, TileCdfSelector, TileCdfWorkUnitBoundary,
    tile_cdf_save_policy,
};
use super::super::{
    SymbolInitBoundary, TileBruPath, TileCoeffFrameFacts, TileCoeffFrameFactsInput,
    TilePayloadSource,
};
use super::*;
use crate::bitstream::tile_payload::encode_symbol_sequence;
use crate::{DecodeLayerSelection, DecodeLimitError, DecodeLimitThreshold, DecodeObuSourceKind};
use splot_core::segment::MAX_SEGMENTS;
use splot_core::span::{ByteOffset, ByteSpan};
use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};
use splot_core::tables::cdf::DEFAULT_Y_MODE_SET_CDF;

const BLOCK_4X4: usize = 0;
const BLOCK_4X8: usize = 1;
const BLOCK_16X16: usize = 6;
const BLOCK_32X32: usize = 9;
const BLOCK_64X64: usize = 12;
const BLOCK_128X128: usize = 15;
const BLOCK_32X8: usize = 22;
const PARTITION_CONTEXT_4X4: usize = 63;

static ROW_4X4: [usize; 64] = [BLOCK_4X4; 64];
static ROW_16X16: [usize; 64] = [BLOCK_16X16; 64];
static ROW_CONTEXT_4X4: [usize; 64] = [PARTITION_CONTEXT_4X4; 64];
static GRID0: [&[usize]; 2] = [&ROW_4X4, &ROW_16X16];
static GRID1: [&[usize]; 2] = [&ROW_4X4, &ROW_16X16];

fn context() -> TilePartitionContextState<'static> {
    TilePartitionContextState::new(
        [&GRID0, &GRID1],
        [&ROW_CONTEXT_4X4, &ROW_CONTEXT_4X4],
        [&ROW_CONTEXT_4X4, &ROW_CONTEXT_4X4],
    )
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
        false,
        TilePartitionLoopRestorationState::NoSyntax,
        PartitionFeatureFlags::new(true, true),
        4,
        true,
        TilePartitionBruState::Active,
    )
    .unwrap()
}

fn frame_level_wiener_ns(unit_size: usize) -> TilePartitionLoopRestorationState {
    TilePartitionLoopRestorationState::Frame(TilePartitionLoopRestorationFrameState::new(
        [
            TilePartitionLoopRestorationPlaneTool::WienerNs,
            TilePartitionLoopRestorationPlaneTool::None,
            TilePartitionLoopRestorationPlaneTool::None,
        ],
        [true, false, false],
        [unit_size, 0, 0],
    ))
}

fn frame_level_chroma_wiener_ns(unit_size: usize) -> TilePartitionLoopRestorationState {
    TilePartitionLoopRestorationState::Frame(TilePartitionLoopRestorationFrameState::new(
        [
            TilePartitionLoopRestorationPlaneTool::None,
            TilePartitionLoopRestorationPlaneTool::WienerNs,
            TilePartitionLoopRestorationPlaneTool::None,
        ],
        [false, true, false],
        [0, unit_size, 0],
    ))
}

fn frame_level_pc_wiener(unit_size: usize) -> TilePartitionLoopRestorationState {
    TilePartitionLoopRestorationState::Frame(TilePartitionLoopRestorationFrameState::new(
        [
            TilePartitionLoopRestorationPlaneTool::PcWiener,
            TilePartitionLoopRestorationPlaneTool::None,
            TilePartitionLoopRestorationPlaneTool::None,
        ],
        [false, false, false],
        [unit_size, 0, 0],
    ))
}

#[derive(Clone, Copy)]
enum LrUnitSymbolRow {
    WienerNs,
    PcWiener,
}

fn lr_unit_symbol_row(work_unit: &DecodeTileWorkUnit<'_>, row: LrUnitSymbolRow) -> [i32; 3] {
    match row {
        LrUnitSymbolRow::WienerNs => *work_unit.cdf().tile_cdfs().rows().use_wiener_ns(),
        LrUnitSymbolRow::PcWiener => *work_unit.cdf().tile_cdfs().rows().use_pc_wiener(),
    }
}

pub(crate) fn make_work_unit(payload: &[u8], update_mode: CdfUpdateMode) -> DecodeTileWorkUnit<'_> {
    make_work_unit_at(payload, update_mode, 0..64, 0..64)
}

pub(crate) fn make_work_unit_at(
    payload: &[u8],
    update_mode: CdfUpdateMode,
    mi_row_range: Range<u32>,
    mi_col_range: Range<u32>,
) -> DecodeTileWorkUnit<'_> {
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
        coeff_frame_facts: TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
            enable_fsc: false,
            enable_idtx_intra: false,
            enable_intra_ist: false,
            enable_inter_ist: false,
            enable_chroma_dctonly: false,
            enable_cctx: false,
            reduced_tx_set: 0,
            lossless_array: [false; MAX_SEGMENTS],
            allow_tcq: false,
            allow_parity_hiding: false,
            base_q_idx: 0,
        }),
        bru_path: TileBruPath::NotUsed,
        symbol: SymbolInitBoundary {
            consumed_bits: payload.len().saturating_mul(8).min(15) as u64,
            symbol_max_bits: payload.len() as i64 * 8 - 15,
            cdf_update_mode: update_mode,
        },
        cdf: TileCdfWorkUnitBoundary::new(
            update_mode,
            tile_cdf_save_policy(TileCdfPolicyInput::single_tile_default(), 0).unwrap(),
            FrameCdfSubset::from_defaults(),
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

fn root_lr_frontier(
    work_unit: &mut DecodeTileWorkUnit<'static>,
    frame: TilePartitionFrameFacts,
) -> Result<TileLoopRestorationRootFrontier, TilePartitionTraversalError> {
    root_lr_frontier_with_limits(work_unit, frame, DecodeLimits::DEFAULT)
}

fn root_lr_frontier_with_limits(
    work_unit: &mut DecodeTileWorkUnit<'static>,
    frame: TilePartitionFrameFacts,
    limits: DecodeLimits,
) -> Result<TileLoopRestorationRootFrontier, TilePartitionTraversalError> {
    consume_tile_loop_restoration_root_frontier(TilePartitionTraversalInput::new(
        work_unit,
        frame,
        context(),
        limits,
    ))
}

fn assert_max_tile_partition_steps_limit(err: &TilePartitionTraversalError, actual: u64) {
    assert!(matches!(
        err,
        TilePartitionTraversalError::Limit(DecodeLimitError::LimitExceeded { .. })
    ));
    if let TilePartitionTraversalError::Limit(DecodeLimitError::LimitExceeded { check }) = err {
        assert_eq!(check.name(), DecodeLimitName::MaxTilePartitionSteps);
        assert_eq!(check.actual(), actual);
    }
}

fn assert_frontier_unsupported(
    frame: TilePartitionFrameFacts,
    expected: TilePartitionTraversalUnsupported,
) {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let err = frontier(&mut work_unit, frame, context()).unwrap_err();

    assert!(matches!(
        err,
        TilePartitionTraversalError::Unsupported(actual) if actual == expected
    ));
}

fn root_call(b_size: usize) -> TilePartitionCall {
    TilePartitionCall::root(0, 0, BlockSize::new(b_size).unwrap(), true)
}

fn assert_child_positions(partition: PartitionType, expected: &[(usize, usize)]) {
    assert_child_positions_for(partition, BLOCK_32X32, expected);
}

fn assert_child_positions_for(
    partition: PartitionType,
    b_size: usize,
    expected: &[(usize, usize)],
) {
    let positions = child_positions_for(partition, b_size);

    assert_eq!(positions.as_slice(), expected);
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
fn chroma_ref_geometry_captures_own_block_until_chroma_offset() {
    let call = root_call(BLOCK_64X64);
    let geometry = call.chroma_ref_geometry();
    assert_eq!(
        (geometry.row, geometry.col, geometry.size.index()),
        (0, 0, BLOCK_64X64)
    );
}

#[test]
fn child_calls_thread_chroma_reference_to_chroma_offset_descendants() {
    let call = root_call(BLOCK_64X64);
    let sub_size = valid_subsize(PartitionType::Horz, call.b_size).unwrap();
    let children = child_calls(
        call,
        PartitionType::Horz,
        sub_size,
        frame(BLOCK_64X64),
        true,
    )
    .unwrap();
    for child in children.as_slice() {
        let geometry = child.chroma_ref_geometry();
        assert_eq!(
            (geometry.row, geometry.col, geometry.size.index()),
            (0, 0, BLOCK_64X64),
            "chroma-offset child must reference the parent §5.20.4.1 chroma geometry"
        );
    }
}

#[test]
fn uv_cfl_context_uses_chroma_reference_base_for_offset_blocks() {
    let mut uv_cfls = TileUvCflState::new(16, 16).unwrap();
    uv_cfls.record_block(0, 4, 4, 4, true);
    let bounds = TilePartitionBounds {
        mi_row_start: 0,
        mi_row_end: 16,
        mi_col_start: 0,
        mi_col_end: 16,
    };
    let chroma_ref = ChromaRefGeometry::new(0, 8, BlockSize::new(BLOCK_16X16).unwrap());

    let ctx = is_cfl_context_for_chroma_ref(&uv_cfls, bounds, chroma_ref);

    assert_eq!(ctx.get(), 1);
}

#[test]
fn shared_mixed_chroma_ref_size_mismatch_forces_inter() {
    let work_unit = make_work_unit(&[0x80], CdfUpdateMode::Enabled);
    let symbols = symbol_decoder_for_work_unit(&work_unit).unwrap();
    let chroma_ref = ChromaRefGeometry::new(28, 184, BlockSize::new(BLOCK_32X8).unwrap());
    let call = TilePartitionCall::child(
        30,
        184,
        BlockSize::new(BLOCK_4X8).unwrap(),
        Some(BlockSize::new(BLOCK_32X8).unwrap()),
        true,
        false,
        PartitionTreeType::Shared,
        Some(chroma_ref),
        true,
        false,
    );

    let frontier =
        decode_block_frontier(call, frame(BLOCK_64X64), call.b_size, true, None, &symbols);

    assert!(frontier.shared_mixed_chroma_ref_forces_inter());
}

#[test]
fn sdp_cfl_allowed_state_tracks_top_luma_and_chroma_partitions() {
    let frame = frame(BLOCK_64X64);
    let mut state = SdpPartitionState::default();
    let luma_root = root_call(BLOCK_64X64).with_tree_type(PartitionTreeType::LumaPart);
    let chroma_root = root_call(BLOCK_64X64).with_tree_type(PartitionTreeType::ChromaPart);

    assert!(state.record_partition(frame, luma_root, PartitionType::Horz4A));
    assert!(state.record_partition(frame, chroma_root, PartitionType::Horz));

    let mut state = SdpPartitionState::default();
    assert!(state.record_partition(frame, luma_root, PartitionType::Horz4A));
    assert!(!state.record_partition(frame, chroma_root, PartitionType::Vert));
}

#[test]
fn sdp_chroma_partition_forced_from_luma_when_chroma_follows_luma() {
    let frame = frame(BLOCK_64X64);
    let luma_root = root_call(BLOCK_64X64).with_tree_type(PartitionTreeType::LumaPart);
    let chroma_root = root_call(BLOCK_64X64).with_tree_type(PartitionTreeType::ChromaPart);

    for partition in [PartitionType::None, PartitionType::Horz] {
        let mut state = SdpPartitionState::default();
        assert!(state.record_partition(frame, luma_root, partition));
        assert_eq!(
            state.forced_chroma_partition(frame, chroma_root),
            Some(partition)
        );
    }
}

#[test]
fn sdp_chroma_partition_not_forced_when_trees_diverge_or_out_of_scope() {
    let frame = frame(BLOCK_64X64);
    let luma_root = root_call(BLOCK_64X64).with_tree_type(PartitionTreeType::LumaPart);
    let chroma_root = root_call(BLOCK_64X64).with_tree_type(PartitionTreeType::ChromaPart);

    let mut state = SdpPartitionState::default();
    state.record_partition(frame, luma_root, PartitionType::Split);
    assert_eq!(state.forced_chroma_partition(frame, chroma_root), None);

    let mut state = SdpPartitionState::default();
    state.record_partition(frame, luma_root, PartitionType::None);
    assert_eq!(state.forced_chroma_partition(frame, luma_root), None);

    let chroma_128 = root_call(BLOCK_128X128).with_tree_type(PartitionTreeType::ChromaPart);
    assert_eq!(state.forced_chroma_partition(frame, chroma_128), None);

    let inter_frame = TilePartitionFrameFacts::new(
        64,
        64,
        BLOCK_64X64,
        3,
        true,
        true,
        false,
        false,
        false,
        false,
        TilePartitionLoopRestorationState::NoSyntax,
        PartitionFeatureFlags::new(true, true),
        4,
        true,
        TilePartitionBruState::Active,
    )
    .unwrap();
    assert_eq!(
        state.forced_chroma_partition(inter_frame, chroma_root),
        None
    );
}

#[test]
fn sdp_cfl_allowed_state_propagates_to_chroma_children() {
    let frame = frame(BLOCK_64X64);
    let chroma_root = root_call(BLOCK_64X64)
        .with_tree_type(PartitionTreeType::ChromaPart)
        .with_cfl_allowed_in_sdp(false);
    let sub_size = valid_subsize(PartitionType::Vert, chroma_root.b_size).unwrap();

    let children = child_calls(chroma_root, PartitionType::Vert, sub_size, frame, false).unwrap();

    assert!(
        children
            .as_slice()
            .iter()
            .all(|child| !child.cfl_allowed_in_sdp)
    );
}

#[test]
fn flat_partition_reaches_root_block_frontier() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);

    let plan = frontier(&mut work_unit, frame(BLOCK_32X32), context()).unwrap();

    assert_eq!(plan.tile_num, 0);
    assert_eq!(plan.steps().len(), 1);
    assert_eq!(plan.steps()[0].decision.partition, PartitionType::None);
    assert_eq!(plan.frontier().r, 0);
    assert_eq!(plan.frontier().c, 0);
    assert_eq!(plan.frontier().b_size.index(), BLOCK_32X32);
    assert_eq!(
        plan.frontier().symbol_checkpoint_before_block.symbol_count,
        1
    );
    assert_eq!(
        plan.frontier()
            .symbol_checkpoint_before_block
            .consumed_bits
            .get(),
        plan.consumed_bits_after
    );
    assert!(plan.pending_children().is_empty());
    assert_eq!(plan.symbol_count_after(), 1);
}

#[test]
fn cursor_returns_live_symbol_state_at_frontier() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);

    let cursor = plan_tile_partition_traversal_cursor(TilePartitionTraversalInput::new(
        &mut work_unit,
        frame(BLOCK_32X32),
        context(),
        DecodeLimits::DEFAULT,
    ))
    .unwrap();
    let (plan, mut symbols) = cursor.into_parts();

    assert_eq!(
        plan.frontier().symbol_checkpoint_before_block,
        symbols.checkpoint()
    );
    assert_eq!(plan.symbol_count_after(), symbols.symbol_count());
    let mut row = DEFAULT_Y_MODE_SET_CDF;
    assert_eq!(symbols.read_symbol(&mut row).unwrap().get(), 0);
    assert_eq!(symbols.symbol_count(), 2);
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
    let cases: &[(PartitionType, &[(usize, usize)])] = &[
        (PartitionType::Horz, &[(0, 0), (4, 0)]),
        (PartitionType::Vert, &[(0, 0), (0, 4)]),
    ];

    for &(partition, expected) in cases {
        assert_child_positions(partition, expected);
    }
    assert_child_positions_for(
        PartitionType::Split,
        BLOCK_128X128,
        &[(0, 0), (0, 16), (16, 0), (16, 16)],
    );
}

#[test]
fn child_call_geometry_follows_spec_order_for_three_way_partitions() {
    let cases: &[(PartitionType, &[(usize, usize)])] = &[
        (PartitionType::Horz3, &[(0, 0), (2, 0), (2, 4), (6, 0)]),
        (PartitionType::Vert3, &[(0, 0), (0, 2), (4, 2), (0, 6)]),
    ];

    for &(partition, expected) in cases {
        assert_child_positions(partition, expected);
    }
}

#[test]
fn child_call_geometry_follows_spec_order_for_four_way_partitions() {
    let cases: &[(PartitionType, &[(usize, usize)])] = &[
        (PartitionType::Horz4A, &[(0, 0), (1, 0), (3, 0), (7, 0)]),
        (PartitionType::Horz4B, &[(0, 0), (1, 0), (5, 0), (7, 0)]),
        (PartitionType::Vert4A, &[(0, 0), (0, 1), (0, 3), (0, 7)]),
        (PartitionType::Vert4B, &[(0, 0), (0, 1), (0, 5), (0, 7)]),
    ];

    for &(partition, expected) in cases {
        assert_child_positions(partition, expected);
    }
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
    static LONG_ROW: [usize; 256] = [PARTITION_CONTEXT_4X4; 256];
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
        None,
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
fn split_child_sdp_recognizes_nested_64x64_shared_roots() {
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

    let child = children.as_slice()[0];

    assert_eq!(child.b_size.index(), BLOCK_64X64);
    assert!(is_intra_sdp_shared_root(facts, child));
}

#[test]
fn extended_sdp_propagates_through_large_parent_to_eligible_child() {
    let root = root_call(BLOCK_128X128);
    let sub_size = valid_subsize(PartitionType::Split, root.b_size).unwrap();
    let mut facts = frame(BLOCK_128X128);
    facts.enable_extended_sdp = true;
    facts.frame_is_intra = false;
    let children = child_calls(root, PartitionType::Split, sub_size, facts, false).unwrap();
    let child = children.as_slice()[0];

    assert_eq!(child.b_size.index(), BLOCK_64X64);
    assert!(child.extended_sdp_allowed);
    assert!(should_read_extended_sdp_region_type(facts, child, PartitionType::Horz).unwrap());
}

#[test]
fn root_sdp_luma_partition_none_reaches_frontier() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut facts = frame(BLOCK_64X64);
    facts.enable_sdp = true;

    let plan = frontier(&mut work_unit, facts, context()).unwrap();

    assert_eq!(plan.steps()[0].decision.partition, PartitionType::None);
    assert_eq!(plan.steps()[0].decision.trace.do_split, Some(false));
    assert_eq!(plan.frontier.b_size.index(), BLOCK_64X64);
    assert!(!plan.frontier.has_chroma);
    assert_eq!(plan.symbol_count_after(), 1);
}

#[test]
fn intra_extended_sdp_sequence_flag_is_inactive_for_traversal() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut facts = frame(BLOCK_32X32);
    facts.enable_extended_sdp = true;

    let plan = frontier(&mut work_unit, facts, context()).unwrap();

    assert_eq!(plan.frontier.b_size.index(), BLOCK_32X32);
    assert_eq!(plan.symbol_count_after(), 1);
}

#[test]
fn inter_extended_sdp_root_does_not_consume_region_type() {
    let payload = encode_symbol_sequence(&[(TileCdfSelector::RegionType { ctx: 2 }, MIXED_REGION)]);
    let mut symbols = SymbolDecoder::with_base_and_config(
        &payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap();
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let mut facts = frame(BLOCK_32X32);
    facts.enable_extended_sdp = true;
    facts.frame_is_intra = false;

    let (call, using_extended_sdp) = read_extended_sdp_region_type(
        facts,
        root_call(BLOCK_32X32),
        PartitionType::Horz,
        &mut cdfs,
        &mut symbols,
    )
    .unwrap();

    assert!(!using_extended_sdp);
    assert_eq!(call.tree_type(), PartitionTreeType::Shared);
    assert!(!call.intra_region);
    assert_eq!(symbols.symbol_count(), 0);
}

#[test]
fn inter_extended_sdp_mixed_child_consumes_region_type() {
    let payload = encode_symbol_sequence(&[(TileCdfSelector::RegionType { ctx: 2 }, MIXED_REGION)]);
    let mut symbols = SymbolDecoder::with_base_and_config(
        &payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap();
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let mut facts = frame(BLOCK_64X64);
    facts.enable_extended_sdp = true;
    facts.frame_is_intra = false;
    let call = TilePartitionCall::child(
        0,
        0,
        BlockSize::new(BLOCK_32X32).unwrap(),
        Some(BlockSize::new(BLOCK_64X64).unwrap()),
        false,
        true,
        PartitionTreeType::Shared,
        None,
        true,
        false,
    );

    let (call, using_extended_sdp) =
        read_extended_sdp_region_type(facts, call, PartitionType::Horz, &mut cdfs, &mut symbols)
            .unwrap();

    assert!(!using_extended_sdp);
    assert_eq!(call.tree_type(), PartitionTreeType::Shared);
    assert!(!call.intra_region);
    assert_eq!(symbols.symbol_count(), 1);
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

fn assert_frame_level_lr_symbol_precedes_partition_read(
    loop_restoration: TilePartitionLoopRestorationState,
    row: LrUnitSymbolRow,
    cdf_update_message: &'static str,
) {
    let mut work_unit = make_work_unit(&[0x00, 0x00, 0x80], CdfUpdateMode::Enabled);
    let before = lr_unit_symbol_row(&work_unit, row);
    let mut facts = frame(BLOCK_32X32);
    facts.loop_restoration = loop_restoration;

    let plan = frontier(&mut work_unit, facts, context()).unwrap();

    assert_eq!(plan.steps().len(), 1);
    assert_eq!(plan.steps()[0].decision.partition, PartitionType::None);
    assert_eq!(plan.steps()[0].symbol_count_before, 1);
    assert_eq!(plan.steps()[0].symbol_count_after, 2);
    assert_eq!(plan.frontier().symbol_count_before_block, 2);
    assert_eq!(
        plan.frontier().symbol_checkpoint_before_block.symbol_count,
        2
    );
    assert_ne!(
        lr_unit_symbol_row(&work_unit, row),
        before,
        "{cdf_update_message}"
    );
}

#[test]
fn frame_level_lr_symbol_precedes_partition_read() {
    let cases = [
        (
            frame_level_wiener_ns(256),
            LrUnitSymbolRow::WienerNs,
            "successful LR unit reads commit the tile-local UseWienerNs CDF row",
        ),
        (
            frame_level_pc_wiener(256),
            LrUnitSymbolRow::PcWiener,
            "successful LR unit reads commit the tile-local UsePcWiener CDF row",
        ),
    ];
    for (loop_restoration, row, message) in cases {
        assert_frame_level_lr_symbol_precedes_partition_read(loop_restoration, row, message);
    }
}

#[test]
fn root_lr_frontier_consumes_only_frame_level_wiener_ns_symbols() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut facts = frame(BLOCK_32X32);
    facts.loop_restoration = frame_level_wiener_ns(256);

    let root = root_lr_frontier(&mut work_unit, facts).unwrap();

    assert_eq!(root.symbol_count_after(), 1);
    assert!(root.consumed_bits_after() > 0);
    assert_eq!(root.lr_units_consumed(), 1);
    assert_eq!(root.active_wiener_ns_units(), 0);
    assert_eq!(
        root.selections(),
        &[WienerNsLrUnitSelection {
            plane: 0,
            unit_row: 0,
            unit_col: 0,
            active: false,
        }]
    );
    assert!(root.active_source_blocks().is_empty());
    assert!(root.all_lr_units_inactive());
}

#[test]
fn root_lr_frontier_reports_active_frame_level_wiener_ns_unit() {
    let mut work_unit = make_work_unit(&[0xFF, 0x00, 0x80], CdfUpdateMode::Enabled);
    let mut facts = frame(BLOCK_32X32);
    facts.loop_restoration = frame_level_wiener_ns(256);

    let root = root_lr_frontier(&mut work_unit, facts).unwrap();

    assert_eq!(root.symbol_count_after(), 1);
    assert_eq!(root.lr_units_consumed(), 1);
    assert_eq!(root.active_wiener_ns_units(), 1);
    assert_eq!(
        root.selections(),
        &[WienerNsLrUnitSelection {
            plane: 0,
            unit_row: 0,
            unit_col: 0,
            active: true,
        }]
    );
    assert_eq!(root.active_source_blocks().len(), 4096);
    assert_eq!(
        root.active_source_blocks()[0],
        WienerNsLrSourceBlock {
            plane: 0,
            row: 0,
            col: 0,
            unit_row: 0,
            unit_col: 0,
            tile_mi_row_start: 0,
            tile_mi_row_end: 64,
            tile_mi_col_start: 0,
            tile_mi_col_end: 64,
            x: 0,
            y: 0,
            width: 4,
            height: 4,
            luma_start_x: 0,
            luma_end_x: 255,
            luma_start_y: 0,
            luma_end_y: 255,
            frame_luma_end_y: 255,
            luma_stripe_start_y: 0,
            luma_stripe_end_y: 55,
        }
    );
    assert!(!root.all_lr_units_inactive());
}

#[test]
fn root_lr_frontier_reports_active_frame_level_pc_wiener_unit() {
    let mut work_unit = make_work_unit(&[0xFF, 0x00, 0x80], CdfUpdateMode::Enabled);
    let mut facts = frame(BLOCK_32X32);
    facts.loop_restoration = frame_level_pc_wiener(256);

    let root = root_lr_frontier(&mut work_unit, facts).unwrap();

    assert_eq!(root.symbol_count_after(), 1);
    assert_eq!(root.lr_units_consumed(), 1);
    assert_eq!(root.active_wiener_ns_units(), 1);
    assert_eq!(
        root.selections(),
        &[WienerNsLrUnitSelection {
            plane: 0,
            unit_row: 0,
            unit_col: 0,
            active: true,
        }]
    );
    assert_eq!(root.active_source_blocks().len(), 4096);
    assert!(!root.all_lr_units_inactive());
}

#[test]
fn wiener_ns_unit_filter_reads_use_bank_for_merged_units_after_bank_growth() {
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let mut state = WienerNsUnitFilterState::default();
    let zero_payload = [0x00; 2048];
    let one_payload = [0xFF; 32];
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled);

    for expected_bank_size in 1..=2 {
        let mut symbols =
            SymbolDecoder::with_base_and_config(&zero_payload, ByteOffset::new(0), config).unwrap();

        read_wiener_ns_unit_filter(1, &mut cdfs, &mut symbols, &mut state).unwrap();

        assert_eq!(state.bank_size[1], expected_bank_size);
    }

    let mut symbols =
        SymbolDecoder::with_base_and_config(&one_payload, ByteOffset::new(0), config).unwrap();
    let before = symbols.symbol_count();

    read_wiener_ns_unit_filter(1, &mut cdfs, &mut symbols, &mut state).unwrap();

    assert_eq!(
        symbols.symbol_count() - before,
        2,
        "merged_param plus one use_bank literal must be consumed for bank_size == 2"
    );
    assert_eq!(state.bank_size[1], 2);
}

#[test]
fn active_lr_source_blocks_track_stripe_bounds() {
    let mut work_unit = make_work_unit(&[0xFF, 0x00, 0x80], CdfUpdateMode::Enabled);
    let mut facts = frame(BLOCK_64X64);
    facts.loop_restoration = frame_level_wiener_ns(256);

    let root = root_lr_frontier(&mut work_unit, facts).unwrap();

    assert_eq!(root.active_source_blocks().len(), 4096);
    let second_stripe = root
        .active_source_blocks()
        .iter()
        .find(|block| block.row == 14 && block.col == 0)
        .unwrap();
    assert_eq!(second_stripe.y, 56);
    assert_eq!(second_stripe.luma_stripe_start_y, 56);
    assert_eq!(second_stripe.luma_stripe_end_y, 119);
}

#[test]
fn active_lr_source_bounds_clamp_to_tile_when_loopfilters_across_tiles_disabled() {
    let mut work_unit =
        make_work_unit_at(&[0xFF, 0x00, 0x80], CdfUpdateMode::Enabled, 0..32, 0..32);
    let mut facts = frame(BLOCK_32X32);
    facts.disable_loopfilters_across_tiles = true;
    facts.loop_restoration = frame_level_wiener_ns(256);

    let root = root_lr_frontier(&mut work_unit, facts).unwrap();

    assert_eq!(root.active_source_blocks().len(), 1024);
    assert_eq!(root.active_source_blocks()[0].luma_end_x, 127);
    assert_eq!(root.active_source_blocks()[0].luma_end_y, 127);
}

#[test]
fn root_lr_frontier_honors_zero_step_limit_before_lr_symbol() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let before = work_unit.cdf().tile_cdfs().clone();
    let mut facts = frame(BLOCK_32X32);
    facts.loop_restoration = frame_level_wiener_ns(256);

    let err = root_lr_frontier_with_limits(
        &mut work_unit,
        facts,
        DecodeLimits::unlimited().with_max_tile_partition_steps(DecodeLimitThreshold::Max(0)),
    )
    .unwrap_err();

    assert_max_tile_partition_steps_limit(&err, 1);
    assert_eq!(work_unit.cdf().tile_cdfs(), &before);
}

#[test]
fn root_lr_frontier_consumes_sdp_chroma_lr_unit() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut facts = frame(BLOCK_64X64);
    facts.enable_sdp = true;
    facts.loop_restoration = frame_level_chroma_wiener_ns(256);

    let frontier = root_lr_frontier(&mut work_unit, facts).unwrap();

    assert_eq!(frontier.lr_units_consumed(), 1);
    assert_eq!(frontier.active_wiener_ns_units(), 0);
    assert_eq!(frontier.selections()[0].plane, 1);
}

#[test]
fn frame_level_wiener_ns_multi_unit_root_counts_every_covered_unit() {
    let mut work_unit = make_work_unit(&[0x00; 12], CdfUpdateMode::Enabled);
    let mut facts = frame(BLOCK_128X128);
    facts.loop_restoration = frame_level_wiener_ns(64);

    let root = root_lr_frontier(&mut work_unit, facts).unwrap();

    assert_eq!(root.symbol_count_after(), 4);
    assert_eq!(root.lr_units_consumed(), 4);
    assert_eq!(root.active_wiener_ns_units(), 0);
    assert_eq!(
        root.selections(),
        &[
            WienerNsLrUnitSelection {
                plane: 0,
                unit_row: 0,
                unit_col: 0,
                active: false,
            },
            WienerNsLrUnitSelection {
                plane: 0,
                unit_row: 0,
                unit_col: 1,
                active: false,
            },
            WienerNsLrUnitSelection {
                plane: 0,
                unit_row: 1,
                unit_col: 0,
                active: false,
            },
            WienerNsLrUnitSelection {
                plane: 0,
                unit_row: 1,
                unit_col: 1,
                active: false,
            },
        ]
    );
    assert!(root.all_lr_units_inactive());
}

#[test]
fn frame_level_wiener_ns_rejects_invalid_unit_size_before_partition_read() {
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut facts = frame(BLOCK_32X32);
    facts.loop_restoration = frame_level_wiener_ns(0);

    let err = frontier(&mut work_unit, facts, context()).unwrap_err();

    assert!(matches!(
        err,
        TilePartitionTraversalError::InvalidLoopRestorationUnitSize {
            plane: 0,
            unit_size: 0
        }
    ));
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
    let mut extended_sdp = frame(BLOCK_32X32);
    extended_sdp.enable_extended_sdp = true;
    extended_sdp.frame_is_intra = false;
    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let plan = frontier(&mut work_unit, extended_sdp, context()).unwrap();
    assert_eq!(plan.symbol_count_after(), 1);

    let mut read_lr = frame(BLOCK_32X32);
    read_lr.loop_restoration = TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
    assert_frontier_unsupported(
        read_lr,
        TilePartitionTraversalUnsupported::ReadLoopRestoration,
    );

    let mut work_unit = make_work_unit(&[0x00, 0x80], CdfUpdateMode::Enabled);
    let mut inter = frame(BLOCK_32X32);
    inter.frame_is_intra = false;
    let plan = frontier(&mut work_unit, inter, context()).unwrap();
    assert_eq!(plan.symbol_count_after(), 1);

    let mut bru = frame(BLOCK_32X32);
    bru.bru_state = TilePartitionBruState::Unsupported;
    assert_frontier_unsupported(bru, TilePartitionTraversalUnsupported::BruOrBridge);
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

    assert_max_tile_partition_steps_limit(&err, 2);
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

#[test]
fn partition_cdf_plane_matches_avm_chroma_part_plane_one() {
    assert_eq!(partition_cdf_plane(PartitionTreeType::Shared), 0);
    assert_eq!(partition_cdf_plane(PartitionTreeType::LumaPart), 0);
    assert_eq!(partition_cdf_plane(PartitionTreeType::ChromaPart), 1);
}
