// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::DrlReorder;
use splot_recon::IDENTITY_WARP_PARAMS;

use super::super::tests::{MI_DIM, N4_32, SB_H4_64, block_at, empty_grid, record_two_wide_column};
use super::super::{
    BlockPrecisionRecord, MAX_WARP_REF_CANDIDATES, MotionMode, Mv, MvBlockContext, MvStack,
    NeighbourMvGrid, SWITCHABLE_FILTERS, find_mv_stack, find_mv_stack_with_temporal,
};
use super::WarpParamBank;

#[test]
fn warp_stack_orders_spatial_before_bank_and_caps_at_four_without_dedup() {
    let mut grid = empty_grid();
    let spatial = [-3_i32 << 16, 5 << 16, 65536 + 1024, -192, 448, 65536 - 2048];
    grid.record_warp_block(
        8,
        0,
        N4_32,
        N4_32,
        0,
        false,
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
        IDENTITY_WARP_PARAMS,
        "out-of-range indices resolve to the identity default"
    );
    assert_warp_candidates(&grid, &block_at(8, 8), IDENTITY_WARP_PARAMS, &bank, &stack);

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
        IDENTITY_WARP_PARAMS,
        "DeriveWrl == 0 leaves the stack default-initialized"
    );
}

fn assert_warp_candidates(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global: [i32; 6],
    bank: &WarpParamBank,
    full: &MvStack,
) {
    for index in 0..=MAX_WARP_REF_CANDIDATES {
        assert_eq!(
            bank.find_candidate(grid, block, global, index),
            full.warp_candidate(index),
            "candidate {index}"
        );
    }
}

#[test]
fn warp_stack_appends_global_motion_after_the_parameter_bank() {
    let mut bank = WarpParamBank::new();
    let older = [1 << 16, 0, 65536 + 64, 0, 0, 65536];
    let newer = [-2 << 16, 7 << 16, 65536 - 128, 320, -640, 65536 + 256];
    let global = [3 << 16, -5 << 16, 65536 + 512, 64, -128, 65536 - 256];
    bank.update(0, older);
    bank.update(0, newer);

    let stack = find_mv_stack_with_temporal(
        &empty_grid(),
        &block_at(8, 8),
        Mv::ZERO,
        global,
        None,
        &bank,
        true,
        DrlReorder::Disabled,
        None,
        None,
        false,
    );

    assert_eq!(stack.warp_candidate(0), newer);
    assert_eq!(stack.warp_candidate(1), older);
    assert_eq!(stack.warp_candidate(2), global);
    assert_eq!(stack.warp_candidate(3), IDENTITY_WARP_PARAMS);
    assert_warp_candidates(&empty_grid(), &block_at(8, 8), global, &bank, &stack);
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
            false,
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
        IDENTITY_WARP_PARAMS,
        "gm/default tail after the corner model"
    );
    assert_warp_candidates(
        &grid,
        &block,
        IDENTITY_WARP_PARAMS,
        &WarpParamBank::new(),
        &stack,
    );
}

#[test]
fn scan_col_feeds_warp_stack() {
    let mut grid = empty_grid();
    let warp_params = [-3_i32 << 16, 5 << 16, 65536 + 1024, -192, 448, 65536 - 2048];
    record_two_wide_column(&mut grid, 6, Mv { row: 0, col: 16 });
    grid.record_warp_block(
        8,
        4,
        2,
        8,
        0,
        false,
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
        IDENTITY_WARP_PARAMS,
        "the tail fills identity defaults after the scan-col inserts"
    );
    assert_warp_candidates(
        &grid,
        &block_at(8, 8),
        IDENTITY_WARP_PARAMS,
        &WarpParamBank::new(),
        &stack,
    );
}
