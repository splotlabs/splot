// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

const WHOLE_TILE: usize = crate::tile::SbRowWindow::WHOLE_TILE_SB_H4;

const SB_N4: usize = 16;

fn joint_mode(value: usize) -> IntraJointMode {
    IntraJointMode::try_new(value).expect("test joint mode")
}

fn primary_mrl() -> MrlSelection {
    MrlSelection::from_symbols(1, Some(0)).expect("primary MRL")
}

fn secondary_mrl() -> MrlSelection {
    MrlSelection::from_symbols(1, Some(1)).expect("secondary MRL")
}

fn state(rows: usize, cols: usize, sb_size4: usize) -> TileIntraFactsState {
    TileIntraFactsState::new_for_tile(0..rows, 0..cols, sb_size4).unwrap()
}

fn luma(joint: IntraJointMode, fsc: u8, use_dip: u8, mrl: MrlSelection) -> IntraFacts {
    IntraFacts::luma(joint, IntraYMode::Dc, 0, fsc, use_dip, mrl)
}

fn record(state: &mut TileIntraFactsState, (r, c): (usize, usize), n4: usize, facts: IntraFacts) {
    state.record_leaf((r, c), (n4, n4), facts, None);
}

fn record_joint(state: &mut TileIntraFactsState, pos: (usize, usize), n4: usize, value: usize) {
    record(state, pos, n4, luma(joint_mode(value), 0, 0, MrlSelection::Disabled));
}

fn record_mrl(state: &mut TileIntraFactsState, pos: (usize, usize), mrl: MrlSelection) {
    record(state, pos, 1, luma(IntraJointMode::DC, 0, 0, mrl));
}

fn joint_modes(state: &TileIntraFactsState, r: usize, c: usize, n4: usize) -> [IntraJointMode; 2] {
    state.neighbours(r, c, n4, n4).joint_modes()
}

#[test]
fn joint_mode_context_counts_directional_neighbours() {
    let empty = state(16, 16, WHOLE_TILE);
    assert_eq!(empty.neighbours(0, 0, 16, 16).y_mode_index_ctx(), 0);
    for (value, ctx) in [(2, 0), (4, 0), (36, 1)] {
        let mut left = state(16, 32, WHOLE_TILE);
        record_joint(&mut left, (0, 0), 16, value);
        assert_eq!(left.neighbours(0, 16, 16, 16).y_mode_index_ctx(), ctx);
    }
    let mut above = state(32, 16, WHOLE_TILE);
    record_joint(&mut above, (0, 0), 16, 36);
    assert_eq!(above.neighbours(16, 0, 16, 16).y_mode_index_ctx(), 1);
    let mut both = state(32, 32, WHOLE_TILE);
    record_joint(&mut both, (0, 16), 16, 36);
    record_joint(&mut both, (16, 0), 16, 36);
    assert_eq!(both.neighbours(16, 16, 16, 16).y_mode_index_ctx(), 2);
}

#[test]
fn unrecorded_block_neighbours_read_as_dc_and_a_reused_row_is_flagged() {
    let mut state = state(64, 64, SB_N4);
    record_joint(&mut state, (0, 0), 16, 36);
    state.grid_mut().enter_row(16);
    assert_eq!(joint_modes(&state, 32, 16, 16), [IntraJointMode::DC; 2]);
    assert_eq!(joint_modes(&state, 16, 0, 16)[1], joint_mode(36));
    assert!(!state.window_violated());
    state.grid_mut().enter_row(48);
    assert_eq!(joint_modes(&state, 16, 0, 16)[1], IntraJointMode::DC);
    assert!(state.window_violated());
}

#[test]
fn an_inter_leaf_records_nothing_but_still_moves_the_window() {
    let mut state = state(64, 64, SB_N4);
    record_joint(&mut state, (0, 0), 16, 36);
    record(&mut state, (16, 0), 16, IntraFacts::NONE);
    assert_eq!(joint_modes(&state, 32, 0, 16)[1], IntraJointMode::DC);
    assert_eq!(joint_modes(&state, 16, 0, 16)[1], joint_mode(36));
    record(&mut state, (48, 0), 16, IntraFacts::NONE);
    assert_eq!(joint_modes(&state, 16, 0, 16)[1], IntraJointMode::DC);
    assert!(state.window_violated());
}

#[test]
fn get_joint_mode_uses_the_spec_neighbour_positions_and_clips_records() {
    let mut state = state(8, 8, WHOLE_TILE);
    record_joint(&mut state, (3, 1), 1, 36);
    assert_eq!(joint_modes(&state, 2, 2, 2)[0], joint_mode(36));
    record_joint(&mut state, (1, 3), 1, 36);
    assert_eq!(joint_modes(&state, 2, 2, 2)[1], joint_mode(36));
    let mut clipped = self::state(4, 4, WHOLE_TILE);
    record_joint(&mut clipped, (2, 2), 16, 36);
    assert_eq!(joint_modes(&clipped, 3, 3, 1), [joint_mode(36); 2]);
}

#[test]
fn empty_dimensions_and_superblock_are_rejected() {
    for (rows, cols, sb_size4) in [(0, 4, SB_N4), (4, 0, SB_N4)] {
        assert!(matches!(
            TileIntraFactsState::new_for_tile(0..rows, 0..cols, sb_size4),
            Err(TileIntraFactsStateError::EmptyDimensions { .. })
        ));
    }
    assert!(matches!(
        TileIntraFactsState::new_for_tile(0..4, 0..4, 0),
        Err(TileIntraFactsStateError::EmptySuperblockSize)
    ));
}

#[test]
fn palette_cache_interleaves_neighbors_preserves_repeats_and_keeps_tail() {
    let mut state = state(4, 4, SB_N4);
    let above = LumaPalette::from_size_symbol(Symbol::new(2), [7, 11, 19, 23, 0, 0, 0, 0]);
    let left = LumaPalette::from_size_symbol(Symbol::new(0), [11, 17, 0, 0, 0, 0, 0, 0]);
    let facts = luma(IntraJointMode::DC, 0, 0, MrlSelection::Disabled);
    state.record_leaf((0, 1), (1, 1), facts, Some(above));
    state.record_leaf((1, 0), (1, 1), facts, Some(left));
    let (cache, len) = state.palette_cache(1, 1);
    assert_eq!(&cache[..len], &[7, 11, 11, 17, 19, 23]);
}

#[test]
fn luma_palette_grid_stores_one_palette_per_block() {
    let mut state = state(4, 4, SB_N4);
    let palette = LumaPalette::from_size_symbol(Symbol::new(1), [7, 11, 19, 0, 0, 0, 0, 0]);
    let facts = luma(IntraJointMode::DC, 0, 0, MrlSelection::Disabled);

    state.record_leaf((1, 0), (2, 2), facts, Some(palette));

    assert_eq!(state.palettes, [palette]);
    assert_eq!(state.palette_at(1, 0), Some(palette));
    assert_eq!(state.palette_at(2, 1), Some(palette));
    assert_eq!(state.palette_at(0, 0), None);
}

#[test]
fn grid_origin_keeps_only_tile_cells() {
    let mut state = TileSegmentIdState::new_for_tile(16..20, 32..37).unwrap();
    assert_eq!(state.grid.cells.len(), 20);

    state.record_block(17, 34, 2, 2, 7);

    assert_eq!(state.cell(17, 34), Some(7));
    assert_eq!(state.cell(18, 35), Some(7));
    assert_eq!(state.cell(15, 34), None);
    assert_eq!(state.cell(17, 37), None);
}

#[test]
fn uses_mrls_neighbours_select_index_and_secondary_contexts() {
    let empty = state(16, 16, SB_N4).neighbours(0, 0, 16, 16);
    assert_eq!((empty.mrl_index_ctx(), empty.mrl_sec_index_ctx()), (0, 0));
    let mut state = state(32, 32, SB_N4);
    record_mrl(&mut state, (7, 11), secondary_mrl());
    record_mrl(&mut state, (11, 7), primary_mrl());

    let neighbours = state.neighbours(8, 8, 4, 4);
    assert_eq!(neighbours.mrl_index_ctx(), 2);
    assert_eq!(neighbours.mrl_sec_index_ctx(), 1);
}

#[test]
fn npos_excludes_above_superblock_row_neighbours() {
    let mut state = state(32, 32, SB_N4);
    for pos in [(31, 15), (15, 31), (15, 16)] {
        let mrl = if pos == (31, 15) {
            primary_mrl()
        } else {
            secondary_mrl()
        };
        record(&mut state, pos, 1, luma(IntraJointMode::DC, 1, 1, mrl));
    }

    let neighbours = state.neighbours(16, 16, 16, 16);
    assert_eq!(neighbours.mrl_index_ctx(), 1);
    assert_eq!(neighbours.mrl_sec_index_ctx(), 0);
    assert_eq!(neighbours.fsc_mode_ctx(), 1);
    assert_eq!(neighbours.use_dip_ctx(), 1);
}

#[test]
fn npos_uses_fallback_positions() {
    let mut state = state(16, 16, SB_N4);
    record_mrl(&mut state, (7, 3), primary_mrl());
    record_mrl(&mut state, (7, 0), secondary_mrl());

    let neighbours = state.neighbours(8, 0, 4, 4);
    assert_eq!(neighbours.mrl_index_ctx(), 2);
    assert_eq!(neighbours.mrl_sec_index_ctx(), 1);
}

#[test]
fn fsc_and_use_dip_neighbours_select_context_sum() {
    let mut state = state(32, 32, SB_N4);
    for pos in [(7, 11), (11, 7)] {
        record(&mut state, pos, 1, luma(IntraJointMode::DC, 1, 1, MrlSelection::Disabled));
    }

    let neighbours = state.neighbours(8, 8, 4, 4);
    assert_eq!(neighbours.fsc_mode_ctx(), 2);
    assert_eq!(neighbours.use_dip_ctx(), 2);
    assert_eq!(state.fsc_mode_at(7, 11), Some(1));
    assert_eq!(state.fsc_mode_at(0, 0), Some(0));
    assert_eq!(state.fsc_mode_at(32, 0), None);
}

#[test]
fn y_mode_facts_record_clip_and_read_back_intrabc_as_dc() {
    let mut state = state(4, 4, WHOLE_TILE);
    let facts = IntraFacts::luma(IntraJointMode::DC, IntraYMode::Dc, -3, 0, 0, MrlSelection::Disabled);
    record(&mut state, (2, 2), 16, facts);
    record(&mut state, (0, 2), 1, IntraFacts::intrabc());

    let expected = Some(TileIntraYModeFacts {
        y_mode: IntraYMode::Dc,
        angle_delta_y: -3,
    });
    assert_eq!(state.y_mode_facts_at(2, 2), expected);
    assert_eq!(state.y_mode_facts_at(3, 3), expected);
    assert_eq!(state.y_mode_facts_at(0, 0), None);
    assert_eq!(state.y_mode_facts_at(4, 4), None);
    assert_eq!(
        state.y_mode_facts_at(0, 2),
        Some(TileIntraYModeFacts {
            y_mode: IntraYMode::Dc,
            angle_delta_y: 0,
        })
    );
}

#[test]
fn smoothness_reads_the_above_and_left_luma_modes() {
    let mut state = state(8, 8, WHOLE_TILE);
    let smooth = IntraFacts::luma(IntraJointMode::DC, IntraYMode::Smooth, 0, 0, 0, MrlSelection::Disabled);
    record(&mut state, (0, 4), 4, smooth);
    record(&mut state, (4, 0), 4, IntraFacts::intrabc());
    assert_eq!(state.neighbours(4, 4, 4, 4).smoothness(), (true, false));
    assert_eq!(state.neighbours(0, 0, 4, 4).smoothness(), (false, false));
}

#[test]
fn uv_cfl_out_of_frame_neighbours_give_context_zero() {
    let state = TileUvCflState::new(16, 16, WHOLE_TILE).unwrap();
    assert_eq!(state.is_cfl_ctx(0, 0, false, false), 0);
}

#[test]
fn uv_cfl_non_cfl_neighbour_keeps_context_zero() {
    let mut state = TileUvCflState::new(16, 32, WHOLE_TILE).unwrap();
    state.record_block(0, 0, 16, 16, false);
    assert_eq!(state.is_cfl_ctx(0, 16, false, true), 0);
}

#[test]
fn uv_cfl_left_neighbour_raises_context_to_one() {
    let mut state = TileUvCflState::new(16, 32, WHOLE_TILE).unwrap();
    state.record_block(0, 0, 16, 16, true);
    assert_eq!(state.is_cfl_ctx(0, 16, false, true), 1);
}

#[test]
fn uv_cfl_above_neighbour_raises_context_to_one() {
    let mut state = TileUvCflState::new(32, 16, WHOLE_TILE).unwrap();
    state.record_block(0, 0, 16, 16, true);
    assert_eq!(state.is_cfl_ctx(16, 0, true, false), 1);
}

#[test]
fn uv_cfl_both_neighbours_raise_context_to_two() {
    let mut state = TileUvCflState::new(32, 32, WHOLE_TILE).unwrap();
    state.record_block(0, 0, 16, 16, true);
    state.record_block(16, 0, 16, 16, true);
    state.record_block(0, 16, 16, 16, true);
    assert_eq!(state.is_cfl_ctx(16, 16, true, true), 2);
}

#[test]
fn uv_cfl_availability_gate_overrides_a_cfl_neighbour() {
    let mut state = TileUvCflState::new(16, 32, WHOLE_TILE).unwrap();
    state.record_block(0, 0, 16, 16, true);
    assert_eq!(state.is_cfl_ctx(0, 16, false, false), 0);
}

#[test]
fn uv_cfl_record_block_clips_to_the_grid_and_rejects_empty_dimensions() {
    let mut state = TileUvCflState::new(4, 4, WHOLE_TILE).unwrap();
    state.record_block(2, 2, 16, 16, true);
    assert_eq!(state.is_cfl_ctx(3, 3, true, true), 2);
    assert_eq!(state.is_cfl_ctx(2, 3, false, true), 1);
    assert!(TileUvCflState::new(0, 4, WHOLE_TILE).is_err());
    assert!(TileUvCflState::new(4, 0, WHOLE_TILE).is_err());
}

/// AV2 § 5.20.5.8 `neg_deinterleave` across its four branches, with asymmetric
/// `(diff, ref, max)` triples so a swapped branch cannot pass by coincidence.
#[test]
fn neg_deinterleave_matches_spec_branches() {
    assert_eq!(neg_deinterleave(3, 0, 8), 3);
    assert_eq!(neg_deinterleave(2, 7, 8), 5);
    assert_eq!(neg_deinterleave(2, 1, 8), 0);
    assert_eq!(neg_deinterleave(1, 1, 8), 2);
    assert_eq!(neg_deinterleave(5, 1, 8), 5);
    assert_eq!(neg_deinterleave(1, 6, 8), 7);
    assert_eq!(neg_deinterleave(2, 6, 8), 5);
    assert_eq!(neg_deinterleave(3, 6, 8), 4);
}

/// AV2 § 5.20.5.8 predictor + § 8.3.2 context: no neighbour => pred 0 / ctx 0;
/// equal up/up-left/left => ctx 2; a differing left is selected as the predictor.
#[test]
fn segment_id_predictor_and_context() {
    let mut state = TileSegmentIdState::new_for_tile(0..4, 0..4).unwrap();
    assert_eq!(state.predictor_and_ctx(0, 0, false, false), (0, 0));
    state.record_block(0, 0, 2, 1, 5);
    state.record_block(0, 1, 1, 1, 5);
    state.record_block(1, 0, 1, 1, 5);
    assert_eq!(state.predictor_and_ctx(1, 1, true, true), (5, 2));
    state.record_block(1, 0, 1, 1, 3);
    assert_eq!(state.predictor_and_ctx(1, 1, true, true), (5, 1));
}

#[test]
fn segment_id_state_translates_tile_coordinates() {
    let mut state = TileSegmentIdState::new_for_tile(4..8, 8..12).unwrap();
    state.record_block(4, 8, 2, 2, 3);
    assert_eq!(state.cell(4, 8), Some(3));
    assert_eq!(state.cell(5, 9), Some(3));
    assert_eq!(state.cell(3, 8), None);
    assert_eq!(state.cell(4, 7), None);
    assert_eq!(state.predictor_and_ctx(5, 9, true, true), (3, 2));
}

#[test]
fn temporal_segment_prediction_context_uses_above_and_left_flags() {
    let mut state = TileSegmentIdState::new_for_tile(0..4, 0..4).unwrap();
    assert_eq!(state.predicted_context(0, 0), 0);
    state.record_predicted(0, 0, 2, 1, true);
    state.record_predicted(1, 0, 1, 2, true);
    assert_eq!(state.predicted_context(1, 1), 2);
    state.record_predicted(1, 0, 1, 1, false);
    assert_eq!(state.predicted_context(1, 1), 1);
}

#[test]
fn frame_segment_map_merges_tiles_and_finds_covered_minimum() {
    let mut frame = FrameSegmentIdMap::new(4, 6).unwrap();
    let mut tile = TileSegmentIdState::new_for_tile(1..4, 2..6).unwrap();
    tile.record_block(1, 2, 2, 2, 5);
    tile.record_block(2, 3, 3, 2, 3);
    frame.merge_tile(&tile);
    assert_eq!(frame.block_min(1, 2, 1, 1), 5);
    assert_eq!(frame.block_min(2, 2, 4, 1), 3);
    assert_eq!(frame.block_min(0, 0, 2, 2), 0);
}
