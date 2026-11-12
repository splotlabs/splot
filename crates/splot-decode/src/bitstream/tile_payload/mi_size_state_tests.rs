// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::super::cdf::block_context::{IntraJointMode, IntraYMode, MrlSelection};
use super::*;
use splot_core::symbol::Symbol;

const BLOCK_4X4: usize = 0;
const BLOCK_8X8: usize = 3;
const BLOCK_16X8: usize = 5;
const BLOCK_64X64: usize = 12;
const BLOCK_256X256: usize = 18;

fn block(index: usize) -> BlockSize {
    BlockSize::new(index).unwrap()
}

fn new_state(mi_rows: usize, mi_cols: usize) -> TileMiSizeState {
    let mut state =
        TileMiSizeState::new_for_tile(0..mi_rows, 0..mi_cols, block(BLOCK_64X64)).unwrap();
    state.enter_sb_row(0);
    state
}

impl TileMiSizeState {
    fn left_mi_size_at(&self, plane: usize, row: usize) -> usize {
        usize::from(self.left_at(plane, row))
    }

    fn above_mi_size_at(&self, plane: usize, col: usize) -> usize {
        usize::from(self.above_plane(plane)[col])
    }
}

#[test]
fn initializes_both_planes_with_clear_context_sentinel() {
    let state = new_state(2, 3);

    for plane in 0..2 {
        for row in 0..2 {
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
    assert_eq!(state.above_plane(0).len(), 16);
}

#[test]
fn allocation_accounting_includes_superblock_padding_and_neighbor_lines() {
    let allocation = TileMiSizeState::allocation(18, 18, block(BLOCK_64X64)).unwrap();

    assert_eq!(allocation.padded_rows(), 32);
    assert_eq!(allocation.padded_cols(), 32);
    assert_eq!(allocation.entry_count(), 2 * 64);
}

#[test]
fn rejects_empty_dimensions() {
    assert!(matches!(
        TileMiSizeState::new_for_tile(0..0, 0..1, block(BLOCK_64X64)).unwrap_err(),
        TileMiSizeStateError::EmptyDimensions {
            mi_rows: 0,
            mi_cols: 1
        }
    ));
    assert!(matches!(
        TileMiSizeState::new_for_tile(0..1, 0..0, block(BLOCK_64X64)).unwrap_err(),
        TileMiSizeStateError::EmptyDimensions {
            mi_rows: 1,
            mi_cols: 0
        }
    ));
}

#[test]
fn updates_luma_footprint_and_neighbor_lines() {
    let mut state = new_state(6, 6);

    state
        .update_luma_block(1, 2, block(BLOCK_16X8), None, None)
        .unwrap();

    for row in 1..3 {
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

    state
        .update_chroma_block(1, 1, block(BLOCK_8X8), 0)
        .unwrap();

    for row in 1..3 {
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

    state.enter_sb_row(16);
    state
        .update_luma_block(16, 16, block(BLOCK_64X64), None, None)
        .unwrap();

    for row in 16..32 {
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
fn rejects_start_before_tile_origin_without_mutating_state() {
    let mut state = TileMiSizeState::new_for_tile(16..32, 16..32, block(BLOCK_64X64)).unwrap();
    let before = state.clone();

    let err = state
        .update_luma_block(0, 16, block(BLOCK_4X4), None, None)
        .unwrap_err();

    assert!(matches!(
        err,
        TileMiSizeStateError::BlockOutOfBounds {
            plane: 0,
            r: 0,
            c: 16
        }
    ));
    assert_eq!(state, before);
}

#[test]
fn rejects_footprint_outside_padded_extent_without_mutating_state() {
    let mut state = new_state(16, 16);
    let before = state.clone();

    let err = state
        .update_luma_block(0, 0, block(BLOCK_256X256), None, None)
        .unwrap_err();

    assert!(matches!(
        err,
        TileMiSizeStateError::BlockOutOfBounds {
            plane: 0,
            r: 0,
            c: 0
        }
    ));
    assert_eq!(state, before);
}

#[test]
fn rejects_coordinate_overflow_without_mutating_state() {
    let mut state = new_state(2, 2);
    let before = state.clone();

    let err = state
        .update_chroma_block(usize::MAX, 0, block(BLOCK_4X4), 0)
        .unwrap_err();

    assert!(matches!(
        err,
        TileMiSizeStateError::BlockOutOfBounds {
            plane: 1,
            r: usize::MAX,
            c: 0
        }
    ));
    assert_eq!(state, before);
}

#[test]
fn partition_neighbours_read_both_planes_after_mutation() {
    let mut state = new_state(16, 16);
    state
        .update_luma_block(0, 0, block(BLOCK_64X64), None, None)
        .unwrap();
    state
        .update_chroma_block(4, 4, block(BLOCK_8X8), 0)
        .unwrap();

    let luma = state.partition_neighbours(0, 0, 0, 8, 8).unwrap();
    assert_eq!(luma.left, [state.left_at(0, 0), state.left_at(0, 8)]);
    assert_eq!(
        luma.above,
        [state.above_plane(0)[0], state.above_plane(0)[8]]
    );
    let chroma = state.partition_neighbours(1, 4, 4, 1, 1).unwrap();
    assert_eq!(chroma.left, [state.left_at(1, 4), state.left_at(1, 5)]);
    assert_eq!(
        chroma.above,
        [state.above_plane(1)[4], state.above_plane(1)[5]]
    );
    assert_eq!(chroma.luma_left, state.left_at(0, 4));
    assert_eq!(chroma.luma_above, state.above_plane(0)[4]);
    assert_eq!(state.partition_neighbours(2, 0, 0, 0, 0), None);
    assert!(state.check_extent(0, 0, 16, 16).is_ok());
    assert!(state.check_extent(0, 0, 17, 16).is_err());
    assert!(state.check_extent(0, 1, 16, 16).is_err());
}

fn square(n4: usize) -> BlockSize {
    block(match n4 {
        1 => BLOCK_4X4,
        2 => BLOCK_8X8,
        4 => 6,
        8 => 9,
        16 => BLOCK_64X64,
        _ => BLOCK_256X256,
    })
}

fn joint(value: usize) -> IntraJointMode {
    IntraJointMode::try_new(value).unwrap()
}

fn facts(joint: IntraJointMode, fsc: u8, use_dip: bool, mrl: MrlSelection) -> IntraFacts {
    IntraFacts::luma(joint, IntraYMode::Dc, 0, fsc, mrl).with_dip(use_dip)
}

fn record(
    state: &mut TileMiSizeState,
    (r, c): (usize, usize),
    n4: usize,
    leaf: Option<IntraFacts>,
) {
    state
        .update_luma_block(r, c, square(n4), leaf, None)
        .unwrap();
}

fn record_joint(state: &mut TileMiSizeState, pos: (usize, usize), n4: usize, value: usize) {
    record(
        state,
        pos,
        n4,
        Some(facts(joint(value), 0, false, MrlSelection::Disabled)),
    );
}

fn mrl(secondary: u8) -> MrlSelection {
    MrlSelection::from_symbols(1, Some(secondary)).unwrap()
}

#[test]
fn joint_modes_read_the_left_bottom_and_above_right_probes() {
    let mut state = TileMiSizeState::new_for_tile(0..32, 0..32, block(BLOCK_256X256)).unwrap();
    assert_eq!(state.intra_neighbours(0, 0, 16, 16).y_mode_index_ctx(), 0);
    record_joint(&mut state, (0, 16), 16, 36);
    record_joint(&mut state, (16, 0), 16, 36);
    let both = state.intra_neighbours(16, 16, 16, 16);
    assert_eq!(both.joint_modes(), [joint(36); 2]);
    assert_eq!(both.y_mode_index_ctx(), 2);
    let mut small = TileMiSizeState::new_for_tile(0..8, 0..8, block(BLOCK_256X256)).unwrap();
    record_joint(&mut small, (3, 1), 1, 36);
    record_joint(&mut small, (1, 3), 1, 4);
    assert_eq!(
        small.intra_neighbours(2, 2, 2, 2).joint_modes(),
        [joint(36), joint(4)]
    );
}

#[test]
fn an_inter_leaf_reads_as_dc_and_a_new_row_clears_the_left_line() {
    let mut state = new_state(64, 64);
    record_joint(&mut state, (0, 0), 16, 36);
    assert_eq!(
        state.intra_neighbours(0, 16, 16, 16).joint_modes()[0],
        joint(36)
    );
    record(&mut state, (16, 0), 16, None);
    let below_inter = state.intra_neighbours(32, 0, 16, 16);
    assert_eq!(below_inter.joint_modes(), [IntraJointMode::DC; 2]);
    state.enter_sb_row(16);
    assert_eq!(
        state.intra_neighbours(16, 16, 16, 16).joint_modes()[0],
        IntraJointMode::DC
    );
}

#[test]
fn probes_past_the_tile_read_as_absent() {
    let mut state = new_state(6, 6);
    let plain = facts(IntraJointMode::DC, 0, false, MrlSelection::Disabled);
    let fsc = facts(IntraJointMode::DC, 1, false, MrlSelection::Disabled);
    assert_eq!(state.intra_neighbours(0, 0, 4, 4).fsc_mode_ctx(), 0);
    record(&mut state, (0, 4), 4, Some(plain));
    record(&mut state, (4, 0), 4, Some(plain));
    record(&mut state, (7, 3), 1, Some(fsc));
    record(&mut state, (3, 7), 1, Some(fsc));
    assert_eq!(state.intra_neighbours(4, 4, 4, 4).fsc_mode_ctx(), 0);
    record(&mut state, (4, 3), 1, Some(fsc));
    assert_eq!(state.intra_neighbours(4, 4, 4, 4).fsc_mode_ctx(), 1);
}

#[test]
fn npos_takes_the_first_two_probes_of_the_superblock_row() {
    let mut state = new_state(32, 32);
    for (pos, secondary) in [((15, 16), 1), ((15, 31), 1), ((31, 15), 0)] {
        let leaf = facts(IntraJointMode::DC, 1, true, mrl(secondary));
        record(&mut state, pos, 1, Some(leaf));
    }
    let across = state.intra_neighbours(16, 16, 16, 16);
    assert_eq!((across.mrl_index_ctx(), across.mrl_sec_index_ctx()), (1, 0));
    assert_eq!((across.fsc_mode_ctx(), across.use_dip_ctx()), (1, 1));
    let mut state = new_state(16, 16);
    for (pos, secondary) in [((7, 3), 0), ((7, 0), 1)] {
        let leaf = facts(IntraJointMode::DC, 0, false, mrl(secondary));
        record(&mut state, pos, 1, Some(leaf));
    }
    let fallback = state.intra_neighbours(8, 0, 4, 4);
    assert_eq!(
        (fallback.mrl_index_ctx(), fallback.mrl_sec_index_ctx()),
        (2, 1)
    );
}

#[test]
fn smoothness_reads_the_above_and_left_luma_modes() {
    let mut state = new_state(8, 8);
    assert_eq!(
        state.intra_neighbours(0, 0, 4, 4).smoothness(),
        (false, false)
    );
    let smooth = IntraFacts::luma(
        IntraJointMode::DC,
        IntraYMode::Smooth,
        0,
        0,
        MrlSelection::Disabled,
    );
    record(&mut state, (0, 4), 4, Some(smooth));
    record(&mut state, (4, 0), 4, Some(IntraFacts::intrabc()));
    assert_eq!(
        state.intra_neighbours(4, 4, 4, 4).smoothness(),
        (true, false)
    );
}

#[test]
fn palette_cache_interleaves_neighbours_and_skips_a_64_row_boundary() {
    let mut state = new_state(32, 32);
    let above = LumaPalette::from_size_symbol(Symbol::new(2), [7, 11, 19, 23, 0, 0, 0, 0]);
    let left = LumaPalette::from_size_symbol(Symbol::new(0), [11, 17, 0, 0, 0, 0, 0, 0]);
    let leaf = Some(facts(IntraJointMode::DC, 0, false, MrlSelection::Disabled));
    state
        .update_luma_block(0, 1, square(1), leaf, Some(above))
        .unwrap();
    state
        .update_luma_block(1, 0, square(1), leaf, Some(left))
        .unwrap();
    let (cache, len) = state.palette_cache(1, 1);
    assert_eq!(&cache[..len], &[7, 11, 11, 17, 19, 23]);
    state
        .update_luma_block(15, 4, square(1), leaf, Some(above))
        .unwrap();
    assert_eq!(state.palette_cache(16, 4).1, 0);
    assert_eq!(state.palettes, [above, left, above]);
}

#[test]
fn uv_bits_ride_on_the_chroma_entries() {
    let mut state = new_state(32, 32);
    assert_eq!(state.uv_edges(0, 0).cfl_ctx(), 0);
    state.update_chroma_block(0, 0, square(4), UV_CFL).unwrap();
    assert_eq!(state.uv_edges(0, 4).cfl_ctx(), 1);
    state
        .update_chroma_block(0, 4, square(4), UV_SMOOTH)
        .unwrap();
    state
        .update_chroma_block(4, 0, square(4), UV_CFL | UV_SMOOTH)
        .unwrap();
    assert_eq!(state.uv_edges(4, 4).cfl_ctx(), 1);
    assert_eq!(state.uv_edges(4, 4).smoothness(), (true, true));
    assert_eq!(
        state.left_at(1, 4) & !(UV_CFL | UV_SMOOTH),
        PARTITION_CONTEXT_LEFT[6]
    );
    state.enter_sb_row(0);
    assert_eq!(state.uv_edges(4, 4).cfl_ctx(), 0);
}
