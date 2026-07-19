// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

const BLOCK_4X4: usize = 0;
const BLOCK_8X8: usize = 3;
const BLOCK_16X8: usize = 5;
const BLOCK_64X64: usize = 12;
const BLOCK_256X256: usize = 18;

fn block(index: usize) -> BlockSize {
    BlockSize::new(index).unwrap()
}

fn new_state(mi_rows: usize, mi_cols: usize) -> TileMiSizeState {
    TileMiSizeState::new(mi_rows, mi_cols, block(BLOCK_64X64)).unwrap()
}

impl TileMiSizeState {
    fn mi_size_at(&self, plane: usize, row: usize, col: usize) -> usize {
        usize::from(self.mi_sizes_plane(plane)[row * self.mi_size_stride + col])
    }

    fn left_mi_size_at(&self, plane: usize, row: usize) -> usize {
        usize::from(self.left_plane(plane)[row])
    }

    fn above_mi_size_at(&self, plane: usize, col: usize) -> usize {
        usize::from(self.above_plane(plane)[col])
    }
}

#[test]
fn initializes_luma_and_chroma_with_clear_context_sentinel() {
    let state = new_state(2, 3);

    for plane in 0..2 {
        for row in 0..2 {
            for col in 0..3 {
                assert_eq!(state.mi_size_at(plane, row, col), BLOCK_256X256);
            }
            assert_eq!(
                state.left_mi_size_at(plane, row),
                usize::from(CLEAR_PARTITION_CONTEXT)
            );
        }
        for col in 0..3 {
            assert_eq!(
                state.above_mi_size_at(plane, col),
                usize::from(CLEAR_PARTITION_CONTEXT)
            );
        }
    }
    assert_eq!(state.mi_sizes_plane(0).len(), 16 * 16);
    assert_eq!(state.mi_size_stride, 16);
    assert_eq!(state.left_plane(0).len(), 16);
    assert_eq!(state.above_plane(0).len(), 16);
}

#[test]
fn allocation_accounting_includes_superblock_padding_and_neighbor_lines() {
    let allocation = TileMiSizeState::allocation(18, 18, block(BLOCK_64X64)).unwrap();

    assert_eq!(allocation.padded_rows(), 32);
    assert_eq!(allocation.padded_cols(), 32);
    assert_eq!(allocation.padded_grid_cells(), 1024);
    assert_eq!(allocation.entry_count(), 2 * (1024 + 32 + 32));
}

#[test]
fn non_square_padded_grid_uses_padded_columns_as_stride() {
    let mut state = new_state(18, 33);

    assert_eq!(state.mi_size_stride, 48);
    assert_eq!(state.mi_sizes_plane(0).len(), 32 * 48);

    state.update_luma_block(16, 32, block(BLOCK_64X64)).unwrap();

    assert_eq!(state.mi_size_at(0, 16, 32), BLOCK_64X64);
    assert_eq!(state.mi_size_at(0, 17, 32), BLOCK_64X64);
    assert_eq!(state.mi_size_at(0, 17, 0), BLOCK_256X256);
}

#[test]
fn rejects_empty_dimensions() {
    assert!(matches!(
        TileMiSizeState::new(0, 1, block(BLOCK_64X64)).unwrap_err(),
        TileMiSizeStateError::EmptyDimensions {
            mi_rows: 0,
            mi_cols: 1
        }
    ));
    assert!(matches!(
        TileMiSizeState::new(1, 0, block(BLOCK_64X64)).unwrap_err(),
        TileMiSizeStateError::EmptyDimensions {
            mi_rows: 1,
            mi_cols: 0
        }
    ));
}

#[test]
fn updates_luma_footprint_and_neighbor_lines() {
    let mut state = new_state(6, 6);

    state.update_luma_block(1, 2, block(BLOCK_16X8)).unwrap();

    for row in 1..3 {
        for col in 2..6 {
            assert_eq!(state.mi_size_at(0, row, col), BLOCK_16X8);
        }
        assert_eq!(
            state.left_mi_size_at(0, row),
            usize::from(partition_context_left(BLOCK_16X8).unwrap())
        );
    }
    for col in 2..6 {
        assert_eq!(
            state.above_mi_size_at(0, col),
            usize::from(partition_context_above(BLOCK_16X8).unwrap())
        );
    }
    assert_eq!(state.mi_size_at(0, 0, 2), BLOCK_256X256);
    assert_eq!(state.mi_size_at(1, 1, 2), BLOCK_256X256);
    assert_eq!(
        state.left_mi_size_at(0, 0),
        usize::from(CLEAR_PARTITION_CONTEXT)
    );
    assert_eq!(
        state.above_mi_size_at(0, 1),
        usize::from(CLEAR_PARTITION_CONTEXT)
    );
}

#[test]
fn updates_chroma_footprint_without_touching_luma() {
    let mut state = new_state(4, 4);

    state.update_chroma_block(1, 1, block(BLOCK_8X8)).unwrap();

    for row in 1..3 {
        for col in 1..3 {
            assert_eq!(state.mi_size_at(1, row, col), BLOCK_8X8);
            assert_eq!(state.mi_size_at(0, row, col), BLOCK_256X256);
        }
        assert_eq!(
            state.left_mi_size_at(1, row),
            usize::from(partition_context_left(BLOCK_8X8).unwrap())
        );
        assert_eq!(
            state.left_mi_size_at(0, row),
            usize::from(CLEAR_PARTITION_CONTEXT)
        );
    }
    for col in 1..3 {
        assert_eq!(
            state.above_mi_size_at(1, col),
            usize::from(partition_context_above(BLOCK_8X8).unwrap())
        );
        assert_eq!(
            state.above_mi_size_at(0, col),
            usize::from(CLEAR_PARTITION_CONTEXT)
        );
    }
}

#[test]
fn accepts_edge_block_footprint_inside_padded_superblock_extent() {
    let mut state = new_state(18, 18);

    state.update_luma_block(16, 16, block(BLOCK_64X64)).unwrap();

    for row in 16..32 {
        for col in 16..32 {
            assert_eq!(state.mi_size_at(0, row, col), BLOCK_64X64);
        }
        assert_eq!(
            state.left_mi_size_at(0, row),
            usize::from(partition_context_left(BLOCK_64X64).unwrap())
        );
    }
    for col in 16..32 {
        assert_eq!(
            state.above_mi_size_at(0, col),
            usize::from(partition_context_above(BLOCK_64X64).unwrap())
        );
    }
}

#[test]
fn rejects_start_outside_visible_dimensions_without_mutating_state() {
    let mut state = new_state(2, 2);
    let before = state.clone();

    let err = state.update_luma_block(2, 0, block(BLOCK_4X4)).unwrap_err();

    assert!(matches!(
        err,
        TileMiSizeStateError::BlockStartOutOfBounds {
            plane: 0,
            r: 2,
            c: 0,
            mi_rows: 2,
            mi_cols: 2
        }
    ));
    assert_eq!(state, before);
}

#[test]
fn rejects_footprint_outside_padded_extent_without_mutating_state() {
    let mut state = new_state(16, 16);
    let before = state.clone();

    let err = state
        .update_luma_block(0, 0, block(BLOCK_256X256))
        .unwrap_err();

    assert!(matches!(
        err,
        TileMiSizeStateError::BlockOutOfBounds {
            plane: 0,
            r: 0,
            c: 0,
            row_end: 64,
            col_end: 64,
            mi_rows: 16,
            mi_cols: 16
        }
    ));
    assert_eq!(state, before);
}

#[test]
fn rejects_coordinate_overflow_without_mutating_state() {
    let mut state = new_state(2, 2);
    let before = state.clone();

    let err = state
        .update_chroma_block(usize::MAX, 0, block(BLOCK_4X4))
        .unwrap_err();

    assert!(matches!(
        err,
        TileMiSizeStateError::CoordinateOverflow {
            coordinate: "row",
            base: usize::MAX,
            offset: 1
        }
    ));
    assert_eq!(state, before);
}

#[test]
fn context_state_view_is_available_after_mutation() {
    let mut state = new_state(16, 16);
    state.update_luma_block(0, 0, block(BLOCK_64X64)).unwrap();
    state.update_chroma_block(4, 4, block(BLOCK_8X8)).unwrap();

    let expected = TilePartitionContextState::new(
        state.mi_sizes_plane(0),
        state.mi_size_stride,
        [state.left_plane(0), state.left_plane(1)],
        [state.above_plane(0), state.above_plane(1)],
    );

    assert_eq!(state.context_state(), expected);
}
