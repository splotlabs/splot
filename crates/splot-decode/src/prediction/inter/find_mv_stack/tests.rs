// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::MotionMode;
use super::*;
use crate::prediction::inter::read_mv::MV_PRECISION_ONE_PEL;
use splot_core::headers::sequence::DrlReorder;

const BLOCK0_MV: Mv = Mv { row: 0, col: 48 };

const N4_32: usize = 8;
const SB_H4_64: usize = 16;
const MI_DIM: usize = 16;

impl MvStack {
    fn num_mv_found(&self) -> usize {
        self.stack.len()
    }
}

fn block_at(mi_row: usize, mi_col: usize) -> MvBlockContext {
    MvBlockContext {
        mi_row,
        mi_col,
        bw4: N4_32,
        bh4: N4_32,
        sb_h4: SB_H4_64,
        ref_frame0: 0,
        ref_frame1: None,
        mi_rows: MI_DIM,
        mi_cols: MI_DIM,
    }
}

fn empty_grid() -> NeighbourMvGrid {
    NeighbourMvGrid::new(MI_DIM, MI_DIM).unwrap()
}

fn record_inter(
    grid: &mut NeighbourMvGrid,
    r: usize,
    c: usize,
    mode: NeighbourYMode,
    mv: Mv,
    skip: bool,
) {
    record_inter_ref(grid, r, c, 0, mode, mv, skip);
}

fn record_inter_ref(
    grid: &mut NeighbourMvGrid,
    r: usize,
    c: usize,
    ref_frame0: i8,
    mode: NeighbourYMode,
    mv: Mv,
    skip: bool,
) {
    grid.record_block(
        r,
        c,
        N4_32,
        N4_32,
        true,
        ref_frame0,
        None,
        mode,
        mv,
        skip,
        SWITCHABLE_FILTERS,
        false,
        BlockPrecisionRecord::default(),
    );
}

fn record_compound_ref(
    grid: &mut NeighbourMvGrid,
    r: usize,
    c: usize,
    ref_frame0: i8,
    ref_frame1: i8,
    masked_compound: bool,
) {
    grid.record_compound_block(
        r,
        c,
        N4_32,
        N4_32,
        ref_frame0,
        ref_frame1,
        false,
        false,
        Mv::ZERO,
        Mv::ZERO,
        false,
        SWITCHABLE_FILTERS,
        false,
        masked_compound,
        CWP_EQUAL,
        false,
        BlockPrecisionRecord::default(),
    );
}

fn record_warp_inter(
    grid: &mut NeighbourMvGrid,
    r: usize,
    c: usize,
    mode: NeighbourYMode,
    mv: Mv,
    skip: bool,
) {
    grid.record_warp_block(
        r,
        c,
        N4_32,
        N4_32,
        0,
        mode,
        mv,
        skip,
        SWITCHABLE_FILTERS,
        false,
        MotionMode::DeltaWarp,
        splot_recon::IDENTITY_WARP_PARAMS,
        BlockPrecisionRecord::default(),
    );
}

#[test]
fn skip_mode_reference_pair_inherits_compound_neighbour_or_keeps_default() {
    let empty = empty_grid();
    let block = block_at(0, N4_32);
    assert_eq!(
        block_neighbour_ctx(&empty, &block).skip_mode_ref_pair((0, 1)),
        (0, 1)
    );

    let mut single = empty_grid();
    record_inter_ref(&mut single, 0, 0, 2, NeighbourYMode::Other, Mv::ZERO, false);
    assert_eq!(
        block_neighbour_ctx(&single, &block).skip_mode_ref_pair((0, 1)),
        (0, 1)
    );

    let mut compound = empty_grid();
    record_compound_ref(&mut compound, 0, 0, 2, 3, false);
    assert_eq!(
        block_neighbour_ctx(&compound, &block).skip_mode_ref_pair((0, 1)),
        (2, 3)
    );
}

#[test]
fn recorded_skip_mode_contributes_to_the_next_block_context() {
    let mut grid = empty_grid();
    grid.record_compound_block(
        0,
        0,
        N4_32,
        N4_32,
        0,
        1,
        false,
        false,
        Mv::ZERO,
        Mv::ZERO,
        true,
        SWITCHABLE_FILTERS,
        false,
        false,
        CWP_EQUAL,
        true,
        BlockPrecisionRecord::default(),
    );
    let ctx = block_neighbour_ctx(&grid, &block_at(0, N4_32));
    assert_eq!(ctx.skip_mode_ctx, 2);
    assert_eq!(ctx.skip_ctx, 2);
}

#[test]
fn tip_neighbour_matches_its_underlying_compound_reference_pair() {
    let mut grid = empty_grid();
    record_inter_ref(
        &mut grid,
        0,
        0,
        TIP_REF_FRAME,
        NeighbourYMode::NewMv,
        Mv::ZERO,
        false,
    );
    let mut block = block_at(0, N4_32);
    block.ref_frame1 = Some(1);

    let without_tip = find_mode_ctx(&grid, &block);
    let with_tip = find_mode_ctx_with_tip(&grid, &block, Some((0, 1)));
    let wrong_pair = find_mode_ctx_with_tip(&grid, &block, Some((0, 2)));

    assert_eq!(without_tip.new_mv_count, 0);
    assert_eq!(wrong_pair.new_mv_count, 0);
    assert_eq!(with_tip.new_mv_count, 2);
    assert_eq!(block_neighbour_ctx(&grid, &block).tip_mode_ctx(), 2);
}

#[test]
fn compound_mv_stack_keeps_paired_vectors_cwp_and_precision_state() {
    let mut grid = empty_grid();
    let mvs = [Mv { row: 8, col: 16 }, Mv { row: -8, col: 24 }];
    let cwp_weight = 12;
    grid.record_compound_block(
        0,
        0,
        N4_32,
        N4_32,
        0,
        1,
        false,
        false,
        mvs[0],
        mvs[1],
        true,
        SWITCHABLE_FILTERS,
        false,
        false,
        cwp_weight,
        true,
        BlockPrecisionRecord::most_probable(MV_PRECISION_ONE_PEL),
    );
    let mut block = block_at(0, N4_32);
    block.ref_frame1 = Some(1);

    let candidate = find_compound_mv_stack_with_temporal(
        &grid,
        &block,
        [Mv::ZERO; 2],
        None,
        DrlReorder::Disabled,
        None,
    )
    .candidate(0);

    assert_eq!(candidate.mvs, mvs);
    assert_eq!(candidate.cwp_weight, cwp_weight);
    assert_eq!(
        block_neighbour_ctx(&grid, &block).most_probable_precision_ctx(),
        2
    );
}

fn grid_with_block0() -> NeighbourMvGrid {
    let mut grid = empty_grid();
    record_inter(&mut grid, 0, 0, NeighbourYMode::NewMv, BLOCK0_MV, true);
    grid
}

#[test]
fn block0_has_no_inter_neighbours_so_context_is_zero() {
    let grid = empty_grid();
    let block0 = block_at(0, 0);

    let ctx = find_mode_ctx(&grid, &block0);
    assert_eq!(ctx.new_mv_context, 0, "top-left NewMvContext");
    assert_eq!(ctx.new_mv_count, 0, "top-left NewMvCount");
    assert_eq!(ctx.warp_mv_count, 0, "top-left WarpMvCount");

    let nctx = block_neighbour_ctx(&grid, &block0);
    assert_eq!(nctx.is_inter_ctx, 0, "no-neighbour is_inter ctx");
    assert_eq!(nctx.skip_mode_ctx, 0, "no-neighbour skip_mode ctx");
    assert_eq!(nctx.skip_ctx, 0, "no-neighbour skip ctx");
    assert!(
        !nctx.has_neighbour,
        "top-left block has no decoded neighbour"
    );

    let stack = find_mv_stack(
        &grid,
        &block0,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(stack.num_mv_found(), 1, "only the zero global-MV fallback");
    assert_eq!(stack.candidate(0), Mv::ZERO, "fallback candidate is zero");
}

#[test]
fn interp_filter_context_uses_matching_reference_neighbours() {
    let mut grid = empty_grid();
    grid.record_block(
        N4_32,
        0,
        N4_32,
        N4_32,
        true,
        0,
        None,
        NeighbourYMode::Other,
        Mv::ZERO,
        false,
        0,
        false,
        BlockPrecisionRecord::default(),
    );
    grid.record_block(
        0,
        N4_32,
        N4_32,
        N4_32,
        true,
        0,
        None,
        NeighbourYMode::Other,
        Mv::ZERO,
        false,
        1,
        false,
        BlockPrecisionRecord::default(),
    );
    let block = block_at(N4_32, N4_32);

    let nctx = block_neighbour_ctx(&grid, &block);
    assert_eq!(
        nctx.interp_filter_ctx(0, false),
        3,
        "two different neighbour filters use the sentinel context"
    );

    let mut one_matching = empty_grid();
    one_matching.record_block(
        N4_32,
        0,
        N4_32,
        N4_32,
        true,
        0,
        None,
        NeighbourYMode::Other,
        Mv::ZERO,
        false,
        1,
        false,
        BlockPrecisionRecord::default(),
    );
    one_matching.record_block(
        0,
        N4_32,
        N4_32,
        N4_32,
        true,
        1,
        None,
        NeighbourYMode::Other,
        Mv::ZERO,
        false,
        0,
        false,
        BlockPrecisionRecord::default(),
    );
    let nctx = block_neighbour_ctx(&one_matching, &block);
    assert_eq!(
        nctx.interp_filter_ctx(0, false),
        1,
        "only neighbours using RefFrame[0] contribute their filter"
    );

    let nctx = block_neighbour_ctx(&empty_grid(), &block_at(0, 0));
    assert_eq!(
        nctx.interp_filter_ctx(0, false),
        SWITCHABLE_FILTERS as usize
    );
    assert_eq!(
        nctx.interp_filter_ctx(0, true),
        usize::from(SWITCHABLE_FILTERS) + INTER_FILTER_COMP_OFFSET
    );
}

#[test]
fn interp_filter_context_suppresses_above_neighbours_at_sb_top() {
    let mut grid = empty_sb_grid();
    grid.record_block(
        0,
        0,
        N4_64,
        N4_64,
        true,
        0,
        None,
        NeighbourYMode::Other,
        BLOCK0_MV,
        false,
        0,
        false,
        BlockPrecisionRecord::default(),
    );
    let block = sb_block_at(N4_64, 0);

    let nctx = block_neighbour_ctx(&grid, &block);
    assert!(
        nctx.has_neighbour,
        "the NPosBuf list keeps the above-superblock neighbour"
    );
    assert_eq!(
        nctx.interp_filter_ctx(0, false),
        SWITCHABLE_FILTERS as usize,
        "the NPos list drops above probes at the superblock top row"
    );
}

#[test]
fn amvd_context_counts_same_reference_amvd_neighbours() {
    let mut grid = empty_grid();
    grid.record_block(
        N4_32,
        0,
        N4_32,
        N4_32,
        true,
        0,
        None,
        NeighbourYMode::NewMv,
        Mv::ZERO,
        false,
        SWITCHABLE_FILTERS,
        true,
        BlockPrecisionRecord::default(),
    );
    grid.record_block(
        0,
        N4_32,
        N4_32,
        N4_32,
        true,
        1,
        None,
        NeighbourYMode::NewMv,
        Mv::ZERO,
        false,
        SWITCHABLE_FILTERS,
        true,
        BlockPrecisionRecord::default(),
    );
    let block = block_at(N4_32, N4_32);

    let nctx = block_neighbour_ctx(&grid, &block);
    assert_eq!(nctx.amvd_ctx(0), 1);
    assert_eq!(nctx.amvd_ctx(1), 1);
    assert_eq!(nctx.amvd_ctx(2), 0);
}

#[test]
fn block1_predicts_block0_mv_via_left_neighbour() {
    let grid = grid_with_block0();
    let block1 = block_at(0, N4_32);

    let ctx = find_mode_ctx(&grid, &block1);
    assert_eq!(ctx.new_mv_count, 2, "both left probes hit the NEWMV block");
    assert_eq!(ctx.new_mv_context, 3, "left-NEWMV NewMvContext");

    let nctx = block_neighbour_ctx(&grid, &block1);
    assert_eq!(
        nctx.is_inter_ctx, 0,
        "all-inter neighbours -> is_inter ctx 0"
    );
    assert_eq!(nctx.skip_ctx, 2, "two skip=1 neighbours -> skip ctx 2");
    assert_eq!(
        nctx.skip_mode_ctx, 0,
        "regular skip neighbours do not affect skip_mode ctx"
    );
    assert!(nctx.has_neighbour, "block 1 has a decoded left neighbour");

    let stack = find_mv_stack(
        &grid,
        &block1,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert!(stack.num_mv_found() >= 1, "at least one candidate");
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "RefMvIdx 0 predicts block 0's MV"
    );
}

#[test]
fn warp_mode_context_counts_matching_warp_neighbours() {
    let mut grid = empty_grid();
    record_warp_inter(&mut grid, 0, 0, NeighbourYMode::Other, BLOCK0_MV, true);
    let block1 = block_at(0, N4_32);

    let ctx = find_mode_ctx(&grid, &block1);
    assert_eq!(
        ctx.warp_mv_count, 2,
        "leftA and leftB both see the warp neighbour"
    );
}

#[test]
fn block2_predicts_block0_mv_via_above_neighbour() {
    let grid = grid_with_block0();
    let block2 = block_at(N4_32, 0);

    let ctx = find_mode_ctx(&grid, &block2);
    assert_eq!(ctx.new_mv_count, 2, "both above probes hit the NEWMV block");
    assert_eq!(ctx.new_mv_context, 3, "above-NEWMV NewMvContext");

    let stack = find_mv_stack(
        &grid,
        &block2,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "RefMvIdx 0 predicts block 0's MV"
    );
}

#[test]
fn block3_predicts_block0_mv_via_above_and_left() {
    let mut grid = grid_with_block0();
    record_inter(&mut grid, 0, N4_32, NeighbourYMode::Other, BLOCK0_MV, true);
    record_inter(&mut grid, N4_32, 0, NeighbourYMode::Other, BLOCK0_MV, true);
    let block3 = block_at(N4_32, N4_32);

    let ctx = find_mode_ctx(&grid, &block3);
    assert_eq!(ctx.new_mv_count, 0, "NEARMV neighbours are not NEW MVs");
    assert_eq!(ctx.new_mv_context, 2, "above+left NEARMV NewMvContext");

    let nctx = block_neighbour_ctx(&grid, &block3);
    assert_eq!(nctx.skip_ctx, 2, "two skip=1 neighbours -> skip ctx 2");

    let stack = find_mv_stack(
        &grid,
        &block3,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "RefMvIdx 0 predicts block 0's MV"
    );
}

#[test]
fn intra_neighbour_does_not_contribute() {
    let mut grid = empty_grid();
    grid.record_block(
        0,
        0,
        N4_32,
        N4_32,
        false,
        -1,
        None,
        NeighbourYMode::Other,
        Mv::ZERO,
        false,
        SWITCHABLE_FILTERS,
        false,
        BlockPrecisionRecord::default(),
    );
    let block1 = block_at(0, N4_32);

    let ctx = find_mode_ctx(&grid, &block1);
    assert_eq!(ctx.new_mv_context, 0, "intra neighbour gives mode ctx 0");

    let nctx = block_neighbour_ctx(&grid, &block1);
    assert_eq!(
        nctx.is_inter_ctx, 3,
        "two intra neighbours -> is_inter ctx 3"
    );

    let stack = find_mv_stack(
        &grid,
        &block1,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(stack.num_mv_found(), 1, "intra neighbour not a candidate");
    assert_eq!(
        stack.candidate(0),
        Mv::ZERO,
        "only the zero global fallback"
    );
}

#[test]
fn mismatched_reference_neighbour_does_not_contribute() {
    let mut grid = empty_grid();
    record_inter_ref(&mut grid, 0, 0, 1, NeighbourYMode::NewMv, BLOCK0_MV, true);
    let block1 = block_at(0, N4_32);

    let ctx = find_mode_ctx(&grid, &block1);
    assert_eq!(ctx.new_mv_context, 0, "ref-mismatch gives mode ctx 0");

    let stack = find_mv_stack(
        &grid,
        &block1,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.num_mv_found(),
        1,
        "ref-mismatch neighbour not a candidate"
    );
}

#[test]
fn single_ref_mode_ctx_matches_compound_list1_without_counting_list0_newmv() {
    let mut grid = empty_grid();
    grid.record_block_with_newmv_lists(
        0,
        0,
        N4_32,
        N4_32,
        1,
        0,
        true,
        false,
        BLOCK0_MV,
        true,
        SWITCHABLE_FILTERS,
        false,
    );
    let block1 = block_at(0, N4_32);

    let ctx = find_mode_ctx(&grid, &block1);
    assert_eq!(
        ctx.new_mv_count, 0,
        "list-1 ref match must not count list-0 NEWMV"
    );
    assert_eq!(
        ctx.new_mv_context, 1,
        "left list-1 match contributes nearestMatch only"
    );
}

#[test]
fn single_ref_mode_ctx_counts_compound_list1_newmv_when_list1_matches() {
    let mut grid = empty_grid();
    grid.record_block_with_newmv_lists(
        0,
        0,
        N4_32,
        N4_32,
        1,
        0,
        false,
        true,
        BLOCK0_MV,
        true,
        SWITCHABLE_FILTERS,
        false,
    );
    let block1 = block_at(0, N4_32);

    let ctx = find_mode_ctx(&grid, &block1);
    assert_eq!(
        ctx.new_mv_count, 2,
        "both left probes count the matching list-1 NEWMV"
    );
    assert_eq!(ctx.new_mv_context, 3, "left NEWMV context");
}

#[test]
fn duplicate_mv_neighbours_merge_to_one_stack_entry() {
    let mut grid = empty_grid();
    record_inter(&mut grid, 0, 0, NeighbourYMode::Other, BLOCK0_MV, true);
    record_inter(&mut grid, N4_32, 0, NeighbourYMode::Other, BLOCK0_MV, true);
    let block3 = block_at(N4_32, N4_32);

    let stack = find_mv_stack(
        &grid,
        &block3,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.num_mv_found(),
        2,
        "deduped neighbour MV + zero fallback"
    );
    assert_eq!(stack.candidate(0), BLOCK0_MV);
    assert_eq!(stack.candidate(1), Mv::ZERO);
}

#[test]
fn distinct_left_and_above_mvs_order_left_before_above() {
    let mut grid = empty_grid();
    record_inter(
        &mut grid,
        0,
        0,
        NeighbourYMode::NewMv,
        Mv { row: 0, col: 64 },
        true,
    );
    record_inter(
        &mut grid,
        0,
        N4_32,
        NeighbourYMode::NewMv,
        Mv { row: 0, col: -32 },
        true,
    );
    record_inter(
        &mut grid,
        N4_32,
        0,
        NeighbourYMode::NewMv,
        Mv { row: 0, col: 32 },
        true,
    );
    let block3 = block_at(N4_32, N4_32);

    let stack = find_mv_stack(
        &grid,
        &block3,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.candidate(0),
        Mv { row: 0, col: 32 },
        "slot 0 = the LEFT neighbour (block 2) MV"
    );
    assert_eq!(
        stack.candidate(1),
        Mv { row: 0, col: -32 },
        "slot 1 = the ABOVE neighbour (block 1) MV"
    );
    assert_eq!(
        stack.candidate(2),
        Mv { row: 0, col: 64 },
        "slot 2 = the corner MV"
    );
    assert_eq!(
        stack.candidate(3),
        Mv::ZERO,
        "slot 3 = the zero global fallback"
    );
    assert_eq!(
        stack.num_mv_found(),
        4,
        "three distinct neighbour MVs + the zero global fallback"
    );
}

#[test]
fn clamp_keeps_small_mvs_unchanged() {
    let grid = grid_with_block0();
    let block1 = block_at(0, N4_32);
    let stack = find_mv_stack(
        &grid,
        &block1,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "clamp leaves the small MV intact"
    );
}

#[test]
fn record_block_marks_every_covered_cell() {
    let grid = grid_with_block0();
    let block1 = block_at(0, N4_32);
    let ctx = find_mode_ctx(&grid, &block1);
    assert_eq!(ctx.new_mv_context, 3, "both left probes see block 0");
}

const N4_64: usize = 16;
const GRID_MI_DIM: usize = 32;

fn sb_block_at(mi_row: usize, mi_col: usize) -> MvBlockContext {
    MvBlockContext {
        mi_row,
        mi_col,
        bw4: N4_64,
        bh4: N4_64,
        sb_h4: SB_H4_64,
        ref_frame0: 0,
        ref_frame1: None,
        mi_rows: GRID_MI_DIM,
        mi_cols: GRID_MI_DIM,
    }
}

fn empty_sb_grid() -> NeighbourMvGrid {
    NeighbourMvGrid::new(GRID_MI_DIM, GRID_MI_DIM).unwrap()
}

fn record_sb(
    grid: &mut NeighbourMvGrid,
    r: usize,
    c: usize,
    mode: NeighbourYMode,
    mv: Mv,
    skip: bool,
) {
    grid.record_block(
        r,
        c,
        N4_64,
        N4_64,
        true,
        0,
        None,
        mode,
        mv,
        skip,
        SWITCHABLE_FILTERS,
        false,
        BlockPrecisionRecord::default(),
    );
}

#[test]
fn second_sb_row_block_predicts_above_sb_mv_across_sb_row_boundary() {
    let mut grid = empty_sb_grid();
    record_sb(&mut grid, 0, 0, NeighbourYMode::NewMv, BLOCK0_MV, true);
    let sb2 = sb_block_at(N4_64, 0);

    let ctx = find_mode_ctx(&grid, &sb2);
    assert_eq!(
        ctx.new_mv_count, 2,
        "both above probes hit SB0 across SB row"
    );
    assert_eq!(ctx.new_mv_context, 3, "above-SB NewMvContext");

    let nctx = block_neighbour_ctx(&grid, &sb2);
    assert!(
        nctx.has_neighbour,
        "the NPosBuf list keeps the decoded above SB across the SB-row boundary"
    );
    assert_eq!(
        nctx.skip_ctx, 2,
        "skip context counts both above-SB NPosBuf neighbours"
    );

    let stack = find_mv_stack(
        &grid,
        &sb2,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "RefMvIdx 0 predicts SB0's MV across the SB-row boundary"
    );
}

#[test]
fn undecoded_later_sb_column_yields_no_candidate() {
    let grid = empty_sb_grid();
    let sb1 = sb_block_at(0, N4_64);

    let nctx = block_neighbour_ctx(&grid, &sb1);
    assert!(
        !nctx.has_neighbour,
        "an SB whose neighbours are all undecoded has no neighbour"
    );

    let stack = find_mv_stack(
        &grid,
        &sb1,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.num_mv_found(),
        1,
        "undecoded neighbours contribute no candidate"
    );
    assert_eq!(
        stack.candidate(0),
        Mv::ZERO,
        "only the zero global-MV fallback"
    );
}

#[test]
fn bottom_right_sb_predicts_from_decoded_above_and_left() {
    let mut grid = empty_sb_grid();
    record_sb(&mut grid, 0, 0, NeighbourYMode::NewMv, BLOCK0_MV, true);
    record_sb(&mut grid, 0, N4_64, NeighbourYMode::Other, BLOCK0_MV, true);
    record_sb(&mut grid, N4_64, 0, NeighbourYMode::Other, BLOCK0_MV, true);
    let sb3 = sb_block_at(N4_64, N4_64);

    let nctx = block_neighbour_ctx(&grid, &sb3);
    assert!(
        nctx.has_neighbour,
        "SB3 has decoded above + left neighbours"
    );

    let stack = find_mv_stack(
        &grid,
        &sb3,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.candidate(0),
        BLOCK0_MV,
        "RefMvIdx 0 predicts the propagated MV in the bottom-right SB"
    );
}

#[test]
fn single_ref_ctx_no_neighbour_is_one() {
    let grid = empty_grid();
    let block0 = block_at(0, 0);
    let nctx = block_neighbour_ctx(&grid, &block0);
    assert!(
        !nctx.has_neighbour,
        "top-left block has no decoded neighbour"
    );
    assert_eq!(
        nctx.single_ref_ctx(0, 2),
        Some(1),
        "no-neighbour single_ref context is 1 (both count_refs are 0)"
    );
}

#[test]
fn single_ref_ctx_counts_a_ref0_neighbour() {
    let grid = grid_with_block0();
    let block1 = block_at(0, N4_32);
    let nctx = block_neighbour_ctx(&grid, &block1);
    assert!(nctx.has_neighbour, "block 1 has a decoded neighbour");
    assert_eq!(
        nctx.single_ref_ctx(0, 2),
        Some(2),
        "a ref-0 neighbour makes thisRefCount > nextRefsCount -> ctx 2"
    );
}

#[test]
fn single_ref_ctx_counts_refs_above_two() {
    let mut grid = empty_grid();
    record_inter_ref(&mut grid, 0, 0, 2, NeighbourYMode::NewMv, Mv::ZERO, true);
    let block = block_at(0, N4_32);
    let nctx = block_neighbour_ctx(&grid, &block);

    assert_eq!(
        nctx.single_ref_ctx(0, 3),
        Some(0),
        "ref-2 neighbour makes ref-0 count lower than later refs"
    );
    assert_eq!(
        nctx.single_ref_ctx(1, 3),
        Some(0),
        "ref-2 neighbour makes ref-1 count lower than later refs"
    );
}

#[test]
fn compound_group_idx_ctx_uses_equal_ref_distance_offset() {
    let grid = empty_grid();
    let nctx = block_neighbour_ctx(&grid, &block_at(0, 0));

    assert_eq!(nctx.compound_group_idx_ctx(false, Some(0)), 0);
    assert_eq!(nctx.compound_group_idx_ctx(true, Some(0)), 6);
}

#[test]
fn compound_group_idx_ctx_combines_left_masked_compound_probes() {
    let mut grid = empty_grid();
    record_compound_ref(&mut grid, 0, 0, 0, 1, true);
    let nctx = block_neighbour_ctx(&grid, &block_at(0, N4_32));

    assert_eq!(nctx.compound_group_idx_ctx(false, None), 3);
}

#[test]
fn compound_group_idx_ctx_combines_left_furthest_future_probes() {
    let mut grid = empty_grid();
    record_inter_ref(&mut grid, 0, 0, 2, NeighbourYMode::Other, Mv::ZERO, false);
    let nctx = block_neighbour_ctx(&grid, &block_at(0, N4_32));

    assert_eq!(nctx.compound_group_idx_ctx(false, Some(2)), 5);
}

#[test]
fn compound_group_idx_ctx_combines_two_nonzero_neighbours() {
    let mut grid = empty_grid();
    record_compound_ref(&mut grid, N4_32, 0, 0, 1, true);
    record_inter_ref(
        &mut grid,
        0,
        N4_32,
        2,
        NeighbourYMode::Other,
        Mv::ZERO,
        false,
    );
    let nctx = block_neighbour_ctx(&grid, &block_at(N4_32, N4_32));

    assert_eq!(nctx.compound_group_idx_ctx(false, Some(2)), 4);
    assert_eq!(nctx.compound_group_idx_ctx(true, Some(2)), 10);
}

#[test]
fn warp_stack_orders_spatial_before_bank_and_caps_at_four_without_dedup() {
    let mut grid = empty_grid();
    let spatial = [-3_i64 << 16, 5 << 16, 65536 + 1024, -192, 448, 65536 - 2048];
    grid.record_warp_block(
        8,
        0,
        N4_32,
        N4_32,
        0,
        NeighbourYMode::Other,
        Mv { row: -8, col: 24 },
        false,
        SWITCHABLE_FILTERS,
        false,
        MotionMode::LocalWarp,
        spatial,
        BlockPrecisionRecord::default(),
    );
    let mut bank = WarpParamBank::new();
    let older = [1 << 16, 0, 65536 + 64, 0, 0, 65536];
    let newer = [-2 << 16, 7 << 16, 65536 - 128, 320, -640, 65536 + 256];
    bank.update(0, older);
    bank.update(0, newer);

    let stack = find_mv_stack(
        &grid,
        &block_at(8, 8),
        Mv::ZERO,
        None,
        &bank,
        true,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(stack.warp_candidate(0), spatial, "first spatial insert");
    assert_eq!(
        stack.warp_candidate(1),
        spatial,
        "the second scan point re-inserts the same neighbour: 7.12.2.11 never dedups"
    );
    assert_eq!(stack.warp_candidate(2), newer, "bank fills newest-first");
    assert_eq!(stack.warp_candidate(3), older, "then the older bank entry");
    assert_eq!(
        stack.warp_candidate(4),
        DEFAULT_WARP_PARAMS,
        "out-of-range indices resolve to the identity default"
    );

    let no_wrl = find_mv_stack(
        &grid,
        &block_at(8, 8),
        Mv::ZERO,
        None,
        &bank,
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        no_wrl.warp_candidate(0),
        DEFAULT_WARP_PARAMS,
        "DeriveWrl == 0 leaves the stack default-initialized"
    );
}

#[test]
fn warp_bank_hit_keeps_the_original_translation_and_evicts_oldest() {
    let mut bank = WarpParamBank::new();
    let base = |trans: i64, scale: i64| [trans, -trans, 65536 + scale, 0, 0, 65536 - scale];
    bank.update(0, base(1 << 16, 64));
    bank.update(0, base(2 << 16, 128));
    bank.update(0, base(9 << 16, 64));
    let mut stack = WarpParamStack::new();
    bank.fill(0, &mut stack);
    assert_eq!(
        stack.slots[0],
        base(1 << 16, 64),
        "params_equal on [2..6) rotates the EXISTING entry to the tail; its translation is kept"
    );
    assert_eq!(stack.slots[1], base(2 << 16, 128));
    assert_eq!(stack.num_found, 2, "the hit did not grow the ring");

    for scale in [192, 256, 320] {
        bank.update(0, base(3 << 16, scale));
    }
    let mut stack = WarpParamStack::new();
    bank.fill(0, &mut stack);
    assert_eq!(stack.num_found, 4, "ring capacity");
    assert_eq!(
        stack.slots[3],
        base(1 << 16, 64),
        "oldest surviving entry after the size-4 ring evicted the front"
    );
}

#[test]
fn warp_bank_clears_per_superblock_row_and_reseeds_per_superblock() {
    let mut grid = NeighbourMvGrid::new(64, 64).unwrap();
    let above = [4 << 16, -6 << 16, 65536 + 512, 64, -128, 65536];
    grid.record_warp_block(
        15,
        4,
        2,
        1,
        0,
        NeighbourYMode::Other,
        Mv { row: 4, col: -12 },
        false,
        SWITCHABLE_FILTERS,
        false,
        MotionMode::ExtendWarp,
        above,
        BlockPrecisionRecord::default(),
    );
    let mut bank = WarpParamBank::new();
    let carried = [7 << 16, 0, 65536 - 320, 0, 0, 65536 + 448];
    bank.reset_for_leaf(&grid, 0, 0, 16);
    bank.update(0, carried);

    bank.reset_for_leaf(&grid, 0, 16, 16);
    let mut stack = WarpParamStack::new();
    bank.fill(0, &mut stack);
    assert_eq!(
        stack.num_found, 1,
        "a same-row superblock transition keeps the bank contents"
    );

    bank.reset_for_leaf(&grid, 16, 0, 16);
    let mut stack = WarpParamStack::new();
    bank.fill(0, &mut stack);
    assert_eq!(
        stack.slots[0], above,
        "the new superblock row cleared contents, then re-seeded the warp neighbour from the row above"
    );
    assert_eq!(
        stack.num_found, 1,
        "carried entry was cleared on the row transition"
    );
}

#[test]
fn corner_derivation_matches_the_hand_computed_model() {
    let mut grid = empty_grid();
    let mut record_cell = |r: usize, c: usize, mv: Mv| {
        grid.record_block(
            r,
            c,
            1,
            1,
            true,
            0,
            None,
            NeighbourYMode::Other,
            mv,
            false,
            SWITCHABLE_FILTERS,
            false,
            BlockPrecisionRecord::default(),
        );
    };
    record_cell(3, 3, Mv { row: 0, col: 0 });
    record_cell(3, 5, Mv { row: 0, col: 8 });
    record_cell(5, 3, Mv { row: 0, col: 0 });
    let block = MvBlockContext {
        mi_row: 4,
        mi_col: 4,
        bw4: 2,
        bh4: 2,
        sb_h4: SB_H4_64,
        ref_frame0: 0,
        ref_frame1: None,
        mi_rows: MI_DIM,
        mi_cols: MI_DIM,
    };
    let stack = find_mv_stack(
        &grid,
        &block,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        true,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.warp_candidate(0),
        [-131_072, 0, 73_728, 0, 0, 65_536],
        "7.12.2.3 finite differences over the three corners (one px extra x-shift across 8 px)"
    );
    assert_eq!(
        stack.warp_candidate(1),
        DEFAULT_WARP_PARAMS,
        "gm/default tail after the corner model"
    );
}

#[test]
fn warp_predicted_mv_projects_the_block_center() {
    let mut bank = WarpParamBank::new();
    let translation = [3 << 16, -2 << 16, 1 << 16, 0, 0, 1 << 16];
    bank.update(0, translation);
    let stack = find_mv_stack(
        &empty_grid(),
        &block_at(4, 8),
        Mv::ZERO,
        None,
        &bank,
        true,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(stack.warp_candidate(0), translation);
    assert_eq!(
        stack.warp_predicted_mv(0, super::super::read_mv::MV_PRECISION_EIGHTH_PEL),
        Mv { row: -16, col: 24 },
        "a pure-translation model projects to its eighth-pel translation"
    );
    assert_eq!(
        stack.warp_predicted_mv(1, super::super::read_mv::MV_PRECISION_EIGHTH_PEL),
        Mv::ZERO,
        "identity default projects to zero"
    );

    let scale = [0, 0, (1 << 16) + 1024, 0, 0, 1 << 16];
    let mut bank = WarpParamBank::new();
    bank.update(0, scale);
    let stack = find_mv_stack(
        &empty_grid(),
        &block_at(4, 8),
        Mv::ZERO,
        None,
        &bank,
        true,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.warp_predicted_mv(0, super::super::read_mv::MV_PRECISION_EIGHTH_PEL),
        Mv { row: 0, col: 6 },
        "xc = 1024 * center_x(47) rounds to 6 eighth-pel at PREC-3"
    );
    assert_eq!(
        stack.warp_predicted_mv(0, super::super::read_mv::MV_PRECISION_HALF_PEL),
        Mv { row: 0, col: 6 },
        "non-eighth precisions round at PREC-2 then double"
    );
}

fn record_unit_inter(grid: &mut NeighbourMvGrid, r: usize, c: usize, mv: Mv) {
    record_unit_inter_with_ref(grid, r, c, 0, mv);
}

fn record_unit_inter_with_ref(
    grid: &mut NeighbourMvGrid,
    r: usize,
    c: usize,
    ref_frame0: i8,
    mv: Mv,
) {
    grid.record_block(
        r,
        c,
        1,
        1,
        true,
        ref_frame0,
        None,
        NeighbourYMode::NewMv,
        mv,
        true,
        SWITCHABLE_FILTERS,
        false,
        BlockPrecisionRecord::default(),
    );
}

#[test]
fn constraint_reorder_promotes_max_weight_nearest_unless_temporal_first() {
    let mut grid = empty_grid();
    let first_mv = Mv { row: 0, col: 8 };
    let heavy_mv = Mv { row: 0, col: 40 };
    record_unit_inter(&mut grid, 11, 7, first_mv);
    record_unit_inter(&mut grid, 7, 11, Mv { row: 0, col: 16 });
    record_unit_inter(&mut grid, 8, 7, Mv { row: 0, col: 24 });
    record_unit_inter(&mut grid, 7, 8, heavy_mv);
    record_unit_inter(&mut grid, 12, 7, heavy_mv);
    record_unit_inter(&mut grid, 7, 12, heavy_mv);
    let block = MvBlockContext {
        mi_row: 8,
        mi_col: 8,
        bw4: 4,
        bh4: 4,
        sb_h4: SB_H4_64,
        ref_frame0: 0,
        ref_frame1: None,
        mi_rows: MI_DIM,
        mi_cols: MI_DIM,
    };

    let sorted = find_mv_stack(
        &grid,
        &block,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Constraint,
        false,
    );
    assert_eq!(
        sorted.num_mv_found(),
        5,
        "four distinct nearest candidates plus the zero global fallback"
    );
    assert_eq!(
        sorted.candidate(0),
        heavy_mv,
        "CONSTRAINT with numNearestCands >= 4 swaps the max-weight entry first"
    );
    assert_eq!(
        sorted.candidate(3),
        first_mv,
        "the displaced slot-0 entry lands in the max-weight slot"
    );

    let unsorted = find_mv_stack(
        &grid,
        &block,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Constraint,
        true,
    );
    assert_eq!(
        unsorted.candidate(0),
        first_mv,
        "useTemporalFirst suppresses the CONSTRAINT reorder"
    );
    assert_eq!(
        unsorted.candidate(3),
        heavy_mv,
        "scan order is preserved when the reorder is suppressed"
    );
}

#[test]
fn temporal_scan_duplicate_weight_can_promote_candidate() {
    let mut grid = empty_grid();
    let first_mv = Mv { row: 0, col: 8 };
    let temporal_mv = Mv { row: 0, col: 40 };
    record_unit_inter(&mut grid, 5, 3, first_mv);
    record_unit_inter(&mut grid, 3, 5, temporal_mv);
    let block = MvBlockContext {
        mi_row: 4,
        mi_col: 4,
        bw4: 2,
        bh4: 2,
        sb_h4: SB_H4_64,
        ref_frame0: 0,
        ref_frame1: None,
        mi_rows: MI_DIM,
        mi_cols: MI_DIM,
    };
    let mut source = TemporalMotionField::new(MI_DIM, MI_DIM).unwrap();
    source.record_block(TemporalMotionBlock {
        mi_row: 4,
        mi_col: 4,
        n4w: 2,
        n4h: 2,
        mi_rows: MI_DIM,
        mi_cols: MI_DIM,
        ref_order_hints: [Some(0), None],
        mvs: [temporal_mv, Mv::ZERO],
        warp_params: [None, None],
    });
    let temporal = TemporalMvContext::from_references(
        (MI_DIM, MI_DIM),
        2,
        1,
        &[0],
        &[true],
        &[1],
        &[Some(source)],
    )
    .unwrap();

    let stack = find_mv_stack_with_temporal(
        &grid,
        &block,
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Always,
        Some(&temporal),
        false,
    );
    assert_eq!(
        stack.candidate(0),
        temporal_mv,
        "temporal duplicate weight should make the matching spatial candidate the DRL winner"
    );
    assert_eq!(
        stack.candidate(1),
        first_mv,
        "the original first spatial candidate is swapped behind the heavier temporal candidate"
    );
}

fn record_two_wide_column(grid: &mut NeighbourMvGrid, c: usize, mv: Mv) {
    grid.record_block(
        8,
        c,
        2,
        8,
        true,
        0,
        None,
        NeighbourYMode::NewMv,
        mv,
        true,
        SWITCHABLE_FILTERS,
        false,
        BlockPrecisionRecord::default(),
    );
}

#[test]
fn scan_col_adds_far_left_candidate_when_column_starts_new_block() {
    let mut grid = empty_grid();
    let left_mv = Mv { row: 0, col: 16 };
    let far_mv = Mv { row: 0, col: 48 };
    record_two_wide_column(&mut grid, 6, left_mv);
    record_two_wide_column(&mut grid, 4, far_mv);

    let stack = find_mv_stack(
        &grid,
        &block_at(8, 8),
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.candidate(0),
        left_mv,
        "nearest scan finds the left block"
    );
    assert_eq!(
        stack.candidate(1),
        far_mv,
        "7.12.2.5 scan col reaches the block starting at deltaCol -3"
    );
    assert_eq!(stack.num_mv_found(), 3, "left + scan-col + zero fallback");
}

#[test]
fn scan_col_skips_undecoded_far_column() {
    let mut grid = empty_grid();
    let left_mv = Mv { row: 0, col: 16 };
    record_two_wide_column(&mut grid, 6, left_mv);

    let stack = find_mv_stack(
        &grid,
        &block_at(8, 8),
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.num_mv_found(),
        2,
        "an unwritten deltaCol -3 column contributes nothing"
    );
}

#[test]
fn scan_col_feeds_warp_stack() {
    let mut grid = empty_grid();
    let warp_params = [-3_i64 << 16, 5 << 16, 65536 + 1024, -192, 448, 65536 - 2048];
    record_two_wide_column(&mut grid, 6, Mv { row: 0, col: 16 });
    grid.record_warp_block(
        8,
        4,
        2,
        8,
        0,
        NeighbourYMode::Other,
        Mv { row: -8, col: 24 },
        false,
        SWITCHABLE_FILTERS,
        false,
        MotionMode::LocalWarp,
        warp_params,
        BlockPrecisionRecord::default(),
    );

    let stack = find_mv_stack(
        &grid,
        &block_at(8, 8),
        Mv::ZERO,
        None,
        &WarpParamBank::new(),
        true,
        DrlReorder::Disabled,
        false,
    );
    assert_eq!(
        stack.warp_candidate(0),
        warp_params,
        "7.12.2.5 scan points insert warp neighbours into the stack"
    );
    assert_eq!(
        stack.warp_candidate(1),
        warp_params,
        "both scan-col rows insert without dedup"
    );
    assert_eq!(
        stack.warp_candidate(2),
        DEFAULT_WARP_PARAMS,
        "the tail fills identity defaults after the scan-col inserts"
    );
}
