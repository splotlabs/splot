// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn active_luma_ccso_state(mi_rows: usize, mi_cols: usize) -> CcsoState {
    let shift = 8 - MI_SIZE_LOG2;
    let grid = ccso_grid(mi_rows, mi_cols, shift, ByteOffset::new(0)).unwrap();
    CcsoState::active(shift, [true, false, false], [false; CCSO_PLANES], grid)
}

#[test]
fn ccso_state_reads_only_at_aligned_origins() {
    let state = active_luma_ccso_state(270, 480);
    assert_eq!(state.shift, 6);
    assert_eq!((state.grid_rows, state.grid_cols), (5, 8));
    let unit_mask = (1usize << state.shift) - 1;
    let aligned = |mi: usize| mi & unit_mask == 0;
    assert!(aligned(0));
    assert!(!aligned(32));
    assert!(aligned(64));
}

#[test]
fn ccso_unit_size_follows_tile_alignment() {
    assert_eq!(
        ccso_mi_width_log2_for_layout(false, 4, 1, 1, &[0, 16], &[0, 16]),
        6
    );
    assert_eq!(
        ccso_mi_width_log2_for_layout(false, 4, 2, 1, &[0, 16, 32], &[0, 16]),
        4
    );
    assert_eq!(
        ccso_mi_width_log2_for_layout(false, 5, 2, 1, &[0, 32, 64], &[0, 16]),
        5
    );
    assert_eq!(
        ccso_mi_width_log2_for_layout(false, 6, 2, 1, &[0, 64, 128], &[0, 16]),
        6
    );
    assert_eq!(
        ccso_mi_width_log2_for_layout(true, 4, 2, 1, &[0, 64, 128], &[0, 16]),
        4
    );
}

#[test]
fn ccso_state_left_neighbour_context_matches_spec_8_3_2() {
    let mut state = active_luma_ccso_state(270, 480);
    assert_eq!(state.block_value(0, 0, 0), 0);
    state
        .set_block_value(0, 0, 0, 1, ByteOffset::new(0))
        .unwrap();
    assert_eq!(state.block_value(0, 0, 0), 1);
    let ctx = 2 * usize::from(state.block_value(0, 0, 0));
    assert_eq!(ctx, 2);
}

#[test]
fn ccso_state_inactive_reads_nothing() {
    let state = CcsoState::inactive();
    assert!(!state.active);
    assert_eq!(state.grid_rows, 0);
}

#[test]
fn ccso_state_rejects_out_of_grid_access() {
    let mut state = active_luma_ccso_state(270, 480);
    assert!(
        state
            .set_block_value(0, state.grid_rows, 0, 1, ByteOffset::new(0))
            .is_err()
    );
    assert!(
        state
            .set_block_value(0, 0, state.grid_cols, 1, ByteOffset::new(0))
            .is_err()
    );
    assert_eq!(state.block_value(0, 0, state.grid_cols), 0);
    assert_eq!(state.block_value(0, 99, 99), 0);
    assert_eq!(state.block_value(0, usize::MAX, 0), 0);
}

#[test]
fn ccso_tile_merge_copies_only_the_owned_region() {
    let offset = ByteOffset::new(0);
    let mut frame = CcsoState::active(4, [true, false, false], [false; CCSO_PLANES], (1, 2, 2));
    // splot-copy-ok: test fixtures need independent tile-local state.
    let mut left = frame.clone();
    // splot-copy-ok: test fixtures need independent tile-local state.
    let mut right = frame.clone();
    left.blocks[0] = vec![1, 7];
    right.blocks[0] = vec![8, 1];

    frame.merge_tile(&left, 0..16, 0..16, offset).unwrap();
    frame.merge_tile(&right, 0..16, 16..32, offset).unwrap();

    assert_eq!(frame.blocks[0], [1, 1]);
}

#[test]
fn first_sb_block_16x64_horz4_partition_matches_avm_tx_16x16() {
    use super::super::{
        MI_SIZE, SelectableLumaTxGrid, TX_PARTITION_HORZ, TX_PARTITION_HORZ4, apply_tx_partition,
        table_usize, tx_size_from_dimensions,
    };
    use splot_core::tables::conversion::MAX_TX_SIZE_RECT;

    const TX_16X64: usize = 17;
    const TX_16X16: usize = 2;
    const TX_16X32: usize = 9;

    assert_eq!(
        table_usize("Max_Tx_Size_Rect", &MAX_TX_SIZE_RECT, 23).unwrap(),
        TX_16X64
    );
    let mut grid = SelectableLumaTxGrid::new(16, 4).unwrap();
    apply_tx_partition(&mut grid, 0, 0, TX_16X64, TX_PARTITION_HORZ4).unwrap();
    let records = grid.records_for_region(0, 0, 16, 4).unwrap();
    assert_eq!(records.len(), 4);
    for record in &records {
        assert_eq!((record.rows, record.cols), (4, 4));
        assert_eq!(
            tx_size_from_dimensions(record.cols * MI_SIZE, record.rows * MI_SIZE),
            Some(TX_16X16)
        );
    }
    let mut wrong = SelectableLumaTxGrid::new(16, 4).unwrap();
    apply_tx_partition(&mut wrong, 0, 0, TX_16X64, TX_PARTITION_HORZ).unwrap();
    assert_eq!(wrong.records_for_region(0, 0, 16, 4).unwrap().len(), 2);
    assert_eq!(tx_size_from_dimensions(16, 32), Some(TX_16X32));
}
