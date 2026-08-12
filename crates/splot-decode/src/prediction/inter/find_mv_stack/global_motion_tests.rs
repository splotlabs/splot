// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

const N4_32: usize = 8;
const MI_DIM: usize = 16;

fn block_at(mi_row: usize, mi_col: usize) -> MvBlockContext {
    MvBlockContext {
        mi_row,
        mi_col,
        bw4: N4_32,
        bh4: N4_32,
        sb_h4: MI_DIM,
        ref_frame0: 0,
        ref_frame1: None,
        mi_rows: MI_DIM,
        mi_cols: MI_DIM,
    }
}

#[test]
fn globalmv_neighbour_uses_current_global_mv_and_retains_list0_model() {
    let mut grid = NeighbourMvGrid::new_for_tile(0..MI_DIM, 0..MI_DIM).unwrap();
    let neighbour_mv = Mv { row: 6, col: 8 };
    let current_global_mv = Mv { row: -4, col: 6 };
    let model = [-73_728, 90_112, 65_152, 3_584, -3_584, 65_152];
    grid.record_warp_block(
        8,
        0,
        N4_32,
        N4_32,
        0,
        false,
        neighbour_mv,
        false,
        SWITCHABLE_FILTERS,
        false,
        MotionMode::Simple,
        model,
        BlockPrecisionRecord::default(),
    );
    let block = block_at(8, 8);

    let stack = find_mv_stack(
        &grid,
        &block,
        current_global_mv,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );

    assert_eq!(stack.candidate(0), current_global_mv);
    assert_ne!(stack.candidate(0), neighbour_mv);
    assert_eq!(stack.candidate_offsets(0), (N4_32 as i32 - 1, -1));
    assert_eq!(
        extend_warp_neighbour_params(&grid, &block, N4_32 as i32 - 1, -1, model),
        Some(model)
    );
}

#[test]
fn global_globalmv_neighbour_uses_current_global_mv_and_retains_list1_model() {
    let mut grid = NeighbourMvGrid::new_for_tile(0..MI_DIM, 0..MI_DIM).unwrap();
    let neighbour_mv1 = Mv { row: -24, col: 40 };
    let current_global_mv = Mv { row: 12, col: -10 };
    let model1 = [32_768, -49_152, 65_280, -1_024, 768, 65_408];
    grid.record_flags(
        8,
        0,
        N4_32,
        N4_32,
        NeighbourFlagSyntax {
            is_inter: true,
            ref_frame0: 0,
            ref_frame1: Some(1),
            motion_mode: MotionMode::Simple,
            ..NON_INTER_FLAG_SYNTAX
        },
    );
    grid.record_motion(
        8,
        0,
        N4_32,
        N4_32,
        NeighbourMotionValues {
            mv: [Mv { row: 8, col: 16 }, neighbour_mv1],
            cwp_weight: CWP_EQUAL,
            stored_warp: None,
            global_mv: [false, true],
            splat_warp: [None, None],
        },
    );
    let mut block = block_at(8, 8);
    block.ref_frame0 = 1;

    let stack = find_mv_stack(
        &grid,
        &block,
        current_global_mv,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        false,
    );

    assert_eq!(stack.candidate(0), current_global_mv);
    assert_ne!(stack.candidate(0), neighbour_mv1);
    assert_eq!(stack.candidate_offsets(0), (N4_32 as i32 - 1, -1));
    assert_eq!(
        extend_warp_neighbour_params(&grid, &block, N4_32 as i32 - 1, -1, model1),
        Some(model1)
    );
}

#[test]
fn nonmatching_globalmv_neighbour_derives_from_its_stored_sub_mv() {
    let mut grid = NeighbourMvGrid::new_for_tile(0..MI_DIM, 0..MI_DIM).unwrap();
    let neighbour_mv = Mv { row: 8, col: 12 };
    grid.record_warp_block(
        0,
        0,
        N4_32,
        N4_32,
        1,
        false,
        neighbour_mv,
        false,
        SWITCHABLE_FILTERS,
        false,
        MotionMode::Simple,
        [-73_728, 90_112, 65_152, 3_584, -3_584, 65_152],
        BlockPrecisionRecord::default(),
    );
    let block = block_at(0, N4_32);
    let frame_size = (MI_DIM * 4, MI_DIM * 4);
    let context = TemporalMvContext::from_references(
        (MI_DIM, MI_DIM),
        10,
        TemporalProjectionConfig {
            frame_size,
            step: 1,
            unit_size8: 8,
            enable_tip: false,
            enable_trajectory: false,
            reduced: false,
        },
        &[0, 1],
        &[true, true],
        &[8, 6],
        &[None, None],
    )
    .unwrap();
    let current_global_mv = Mv { row: -40, col: 80 };

    let stack = find_mv_stack_with_temporal(
        &grid,
        &block,
        current_global_mv,
        DEFAULT_WARP_PARAMS,
        None,
        &WarpParamBank::new(),
        false,
        DrlReorder::Disabled,
        None,
        Some(context.order_hint_mv_context()),
        false,
    );

    assert_eq!(stack.candidate(0), Mv { row: 4, col: 6 });
    assert_ne!(stack.candidate(0), Mv { row: -20, col: 40 });
}
