// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

fn cdef_state(rows: usize, cols: usize, sb_size4: usize) -> CdefState {
    CdefState {
        row_start: 0,
        col_start: 0,
        rows,
        cols,
        values: vec![None; rows * cols],
        sb_size4,
    }
}

#[test]
fn cdef_index0_context_uses_zero_strength_neighbours_in_same_superblock() {
    let mut state = cdef_state(4, 4, 32);

    assert_eq!(state.cdef_index0_ctx_at(0, 0, 0, 0).unwrap(), 0);

    let left = state.index(0, 0).unwrap();
    state.values[left] = Some(0);
    assert_eq!(state.cdef_index0_ctx_at(0, 16, 0, 0).unwrap(), 2);

    let above = state.index(0, 1).unwrap();
    state.values[above] = Some(0);
    let left = state.index(1, 0).unwrap();
    state.values[left] = Some(0);
    assert_eq!(state.cdef_index0_ctx_at(16, 16, 0, 0).unwrap(), 3);

    state.values[left] = Some(2);
    assert_eq!(state.cdef_index0_ctx_at(16, 16, 0, 0).unwrap(), 1);

    let mut state = cdef_state(4, 4, 32);
    let above = state.index(1, 1).unwrap();
    state.values[above] = Some(0);
    assert_eq!(state.cdef_index0_ctx_at(32, 16, 0, 0).unwrap(), 0);
}

#[test]
fn cdef_index0_context_stops_at_tile_start() {
    let mut state = cdef_state(4, 4, 32);
    let left = state.index(0, 0).unwrap();
    state.values[left] = Some(0);
    let above = state.index(0, 1).unwrap();
    state.values[above] = Some(0);

    assert_eq!(state.cdef_index0_ctx_at(0, 16, 0, 16).unwrap(), 0);
    let left = state.index(1, 0).unwrap();
    state.values[left] = Some(0);
    assert_eq!(state.cdef_index0_ctx_at(16, 16, 16, 0).unwrap(), 2);
}

#[test]
fn cdef_fill_units_uses_cdef_aligned_origin_and_block_extent() {
    let mut state = cdef_state(4, 4, 32);

    state.fill_units(8, 8, 32, 32, 5).unwrap();

    assert_eq!(state.value(0, 0).unwrap(), Some(5));
    assert_eq!(state.value(0, 1).unwrap(), Some(5));
    assert_eq!(state.value(1, 0).unwrap(), Some(5));
    assert_eq!(state.value(1, 1).unwrap(), Some(5));
    assert_eq!(state.value(2, 0).unwrap(), None);
    assert_eq!(state.value(0, 2).unwrap(), None);
}

#[test]
fn cdef_tile_merge_copies_only_the_owned_region() {
    let mut frame = cdef_state(1, 2, 16);
    let offset = ByteOffset::new(0);
    let mut left = frame.try_for_tile(0..16, 0..16, offset).unwrap();
    let mut right = frame.try_for_tile(0..16, 16..32, offset).unwrap();
    left.values = vec![Some(1)];
    right.values = vec![Some(2)];

    frame.merge_tile(&left, 0..16, 0..16, offset).unwrap();
    frame.merge_tile(&right, 0..16, 16..32, offset).unwrap();

    assert_eq!(frame.values, [Some(1), Some(2)]);
}
