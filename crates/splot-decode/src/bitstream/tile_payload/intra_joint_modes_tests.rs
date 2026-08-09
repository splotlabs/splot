// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

const D135_JOINT_MODE: u8 = 36;
const SMOOTH_V_JOINT_MODE: u8 = 2;
const SB_N4: usize = 16;

#[test]
fn out_of_frame_neighbours_give_context_zero() {
    let state = TileIntraJointModeState::new(16, 16).unwrap();
    assert_eq!(state.y_mode_index_ctx(0, 0, 16, 16), 0);
}

#[test]
fn non_directional_neighbour_keeps_context_zero() {
    let mut state = TileIntraJointModeState::new(16, 32).unwrap();
    state.record_block(0, 0, 16, 16, SMOOTH_V_JOINT_MODE);
    assert_eq!(state.y_mode_index_ctx(0, 16, 16, 16), 0);
}

#[test]
fn directional_left_neighbour_raises_context_to_one() {
    let mut state = TileIntraJointModeState::new(16, 32).unwrap();
    state.record_block(0, 0, 16, 16, D135_JOINT_MODE);
    assert_eq!(state.y_mode_index_ctx(0, 16, 16, 16), 1);
}

#[test]
fn directional_above_neighbour_raises_context_to_one() {
    let mut state = TileIntraJointModeState::new(32, 16).unwrap();
    state.record_block(0, 0, 16, 16, D135_JOINT_MODE);
    assert_eq!(state.y_mode_index_ctx(16, 0, 16, 16), 1);
}

#[test]
fn directional_both_neighbours_raise_context_to_two() {
    let mut state = TileIntraJointModeState::new(32, 32).unwrap();
    state.record_block(0, 16, 16, 16, D135_JOINT_MODE);
    state.record_block(16, 0, 16, 16, D135_JOINT_MODE);
    assert_eq!(state.y_mode_index_ctx(16, 16, 16, 16), 2);
}

#[test]
fn non_intra_block_resets_directional_neighbour_to_dc() {
    let mut state = TileIntraJointModeState::new(64, 64).unwrap();
    state.record_block(32, 0, 16, 16, D135_JOINT_MODE);
    state.record_block(16, 16, 16, 16, D135_JOINT_MODE);
    assert_eq!(state.y_mode_index_ctx(32, 16, 16, 16), 2);

    state.record_non_intra_block(32, 0, 16, 16);
    state.record_non_intra_block(16, 16, 16, 16);
    assert_eq!(state.neighbour_joint_modes(32, 16, 16, 16), [0, 0]);
    assert_eq!(state.y_mode_index_ctx(32, 16, 16, 16), 0);
}

#[test]
fn get_joint_mode_uses_the_spec_neighbour_positions() {
    let mut state = TileIntraJointModeState::new(8, 8).unwrap();
    state.record_block(3, 1, 1, 1, D135_JOINT_MODE);
    assert_eq!(state.get_joint_mode(0, 2, 2, 2, 2), D135_JOINT_MODE);
    state.record_block(1, 3, 1, 1, D135_JOINT_MODE);
    assert_eq!(state.get_joint_mode(1, 2, 2, 2, 2), D135_JOINT_MODE);
}

#[test]
fn last_non_directional_mode_does_not_raise_the_context() {
    let mut state = TileIntraJointModeState::new(16, 32).unwrap();
    state.record_block(0, 0, 16, 16, NON_DIRECTIONAL_MODES_COUNT - 1);
    assert_eq!(state.y_mode_index_ctx(0, 16, 16, 16), 0);
}

#[test]
fn empty_dimensions_are_rejected() {
    assert!(matches!(
        TileIntraJointModeState::new(0, 4),
        Err(TileIntraJointModeStateError::EmptyDimensions { .. })
    ));
    assert!(matches!(
        TileIntraJointModeState::new(4, 0),
        Err(TileIntraJointModeStateError::EmptyDimensions { .. })
    ));
}

#[test]
fn record_block_clips_to_the_grid() {
    let mut state = TileIntraJointModeState::new(4, 4).unwrap();
    state.record_block(2, 2, 16, 16, D135_JOINT_MODE);
    assert_eq!(state.get_joint_mode(0, 2, 3, 1, 1), D135_JOINT_MODE);
}

#[test]
fn luma_palette_grid_stores_one_palette_per_block() {
    let mut state = TileLumaPaletteState::new(4, 4, SB_N4).unwrap();
    let palette = LumaPalette::new(3, [7, 11, 19, 0, 0, 0, 0, 0]).unwrap();

    state.record_block(1, 0, 2, 2, Some(palette));

    assert_eq!(state.palettes, [palette]);
    assert_eq!(state.palette_at(1, 0), Some(palette));
    assert_eq!(state.palette_at(2, 1), Some(palette));
    assert_eq!(state.palette_at(0, 0), None);
}

#[test]
fn luma_palette_grid_cell_stays_pointer_sized() {
    assert_eq!(size_of::<Option<NonZeroUsize>>(), size_of::<usize>());
    assert!(size_of::<Option<NonZeroUsize>>() < size_of::<Option<LumaPalette>>());
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
fn uses_mrls_out_of_frame_neighbours_give_context_zero() {
    let state = TileUsesMrlsState::new(16, 16, SB_N4).unwrap();

    assert_eq!(state.mrl_index_ctx(0, 0, 16, 16), 0);
    assert_eq!(state.mrl_sec_index_ctx(0, 0, 16, 16), 0);
}

#[test]
fn uses_mrls_neighbours_select_index_and_secondary_contexts() {
    let mut state = TileUsesMrlsState::new(32, 32, SB_N4).unwrap();
    state.record_block(7, 11, 1, 1, 2);
    state.record_block(11, 7, 1, 1, 1);

    assert_eq!(state.neighbour_uses_mrls(8, 8, 4, 4), [1, 2]);
    assert_eq!(state.mrl_index_ctx(8, 8, 4, 4), 2);
    assert_eq!(state.mrl_sec_index_ctx(8, 8, 4, 4), 1);
}

#[test]
fn uses_mrls_npos_excludes_above_superblock_row_neighbours() {
    let mut state = TileUsesMrlsState::new(32, 32, SB_N4).unwrap();
    state.record_block(31, 15, 1, 1, 1);
    state.record_block(15, 31, 1, 1, 2);
    state.record_block(15, 16, 1, 1, 2);

    assert_eq!(state.neighbour_uses_mrls(16, 16, 16, 16), [1, 0]);
    assert_eq!(state.mrl_index_ctx(16, 16, 16, 16), 1);
    assert_eq!(state.mrl_sec_index_ctx(16, 16, 16, 16), 0);
}

#[test]
fn uses_mrls_npos_uses_fallback_positions() {
    let mut state = TileUsesMrlsState::new(16, 16, SB_N4).unwrap();
    state.record_block(7, 3, 1, 1, 1);
    state.record_block(7, 0, 1, 1, 2);

    assert_eq!(state.neighbour_uses_mrls(8, 0, 4, 4), [1, 2]);
    assert_eq!(state.mrl_index_ctx(8, 0, 4, 4), 2);
    assert_eq!(state.mrl_sec_index_ctx(8, 0, 4, 4), 1);
}

#[test]
fn uses_mrls_record_block_clips_to_the_grid() {
    let mut state = TileUsesMrlsState::new(4, 4, SB_N4).unwrap();
    state.record_block(2, 2, 16, 16, 2);

    assert_eq!(state.neighbour_uses_mrls(2, 3, 1, 1), [2, 0]);
    assert_eq!(state.mrl_index_ctx(0, 0, 1, 1), 0);
}

#[test]
fn uses_mrls_empty_dimensions_are_rejected() {
    assert!(matches!(
        TileUsesMrlsState::new(0, 4, SB_N4),
        Err(TileUsesMrlsStateError::EmptyDimensions { .. })
    ));
    assert!(matches!(
        TileUsesMrlsState::new(4, 0, SB_N4),
        Err(TileUsesMrlsStateError::EmptyDimensions { .. })
    ));
    assert!(matches!(
        TileUsesMrlsState::new(4, 4, 0),
        Err(TileUsesMrlsStateError::EmptySuperblockSize)
    ));
}

fn record_fsc_and_use_dip(
    fsc: &mut TileFscModeState,
    use_dip: &mut TileUseDipState,
    block_r: usize,
    block_c: usize,
) {
    fsc.record_block(block_r, block_c, 1, 1, 1);
    use_dip.record_block(block_r, block_c, 1, 1, 1);
}

#[test]
fn fsc_and_use_dip_neighbours_select_context_sum() {
    let mut state = TileFscModeState::new(32, 32, SB_N4).unwrap();
    let mut use_dip = TileUseDipState::new(32, 32, SB_N4).unwrap();
    record_fsc_and_use_dip(&mut state, &mut use_dip, 7, 11);
    record_fsc_and_use_dip(&mut state, &mut use_dip, 11, 7);

    assert_eq!(state.fsc_mode_ctx(8, 8, 4, 4), 2);
    assert_eq!(use_dip.use_dip_ctx(8, 8, 4, 4), 2);
}

#[test]
fn fsc_and_use_dip_npos_excludes_above_superblock_row_neighbours() {
    let mut state = TileFscModeState::new(32, 32, SB_N4).unwrap();
    let mut use_dip = TileUseDipState::new(32, 32, SB_N4).unwrap();
    record_fsc_and_use_dip(&mut state, &mut use_dip, 31, 15);
    record_fsc_and_use_dip(&mut state, &mut use_dip, 15, 31);
    record_fsc_and_use_dip(&mut state, &mut use_dip, 15, 16);

    assert_eq!(state.fsc_mode_ctx(16, 16, 16, 16), 1);
    assert_eq!(use_dip.use_dip_ctx(16, 16, 16, 16), 1);
}

#[test]
fn fsc_and_use_dip_empty_dimensions_are_rejected() {
    assert!(matches!(
        TileFscModeState::new(0, 4, SB_N4),
        Err(TileFscModeStateError::EmptyDimensions { .. })
    ));
    assert!(matches!(
        TileUseDipState::new(0, 4, SB_N4),
        Err(TileUseDipStateError::EmptyDimensions { .. })
    ));
    assert!(matches!(
        TileFscModeState::new(4, 0, SB_N4),
        Err(TileFscModeStateError::EmptyDimensions { .. })
    ));
    assert!(matches!(
        TileUseDipState::new(4, 0, SB_N4),
        Err(TileUseDipStateError::EmptyDimensions { .. })
    ));
    assert!(matches!(
        TileFscModeState::new(4, 4, 0),
        Err(TileFscModeStateError::EmptySuperblockSize)
    ));
    assert!(matches!(
        TileUseDipState::new(4, 4, 0),
        Err(TileUseDipStateError::EmptySuperblockSize)
    ));
}

#[test]
fn y_mode_state_records_and_clips_blocks() {
    let mut state = TileIntraYModeState::new(4, 4).unwrap();
    state.record_block(2, 2, 16, 16, IntraYMode::DC_PRED, -3);

    let expected = Some(TileIntraYModeFacts {
        y_mode: IntraYMode::DC_PRED,
        angle_delta_y: -3,
    });
    assert_eq!(state.y_mode_facts_at(2, 2), expected);
    assert_eq!(state.y_mode_facts_at(3, 3), expected);
    assert_eq!(state.y_mode_facts_at(0, 0), None);
    assert_eq!(state.y_mode_facts_at(4, 4), None);
}

#[test]
fn uv_cfl_out_of_frame_neighbours_give_context_zero() {
    let state = TileUvCflState::new(16, 16).unwrap();
    assert_eq!(state.is_cfl_ctx(0, 0, false, false), 0);
}

#[test]
fn uv_cfl_non_cfl_neighbour_keeps_context_zero() {
    let mut state = TileUvCflState::new(16, 32).unwrap();
    state.record_block(0, 0, 16, 16, false);
    assert_eq!(state.is_cfl_ctx(0, 16, false, true), 0);
}

#[test]
fn uv_cfl_left_neighbour_raises_context_to_one() {
    let mut state = TileUvCflState::new(16, 32).unwrap();
    state.record_block(0, 0, 16, 16, true);
    assert_eq!(state.is_cfl_ctx(0, 16, false, true), 1);
}

#[test]
fn uv_cfl_above_neighbour_raises_context_to_one() {
    let mut state = TileUvCflState::new(32, 16).unwrap();
    state.record_block(0, 0, 16, 16, true);
    assert_eq!(state.is_cfl_ctx(16, 0, true, false), 1);
}

#[test]
fn uv_cfl_both_neighbours_raise_context_to_two() {
    let mut state = TileUvCflState::new(32, 32).unwrap();
    state.record_block(0, 0, 16, 16, true);
    state.record_block(16, 0, 16, 16, true);
    state.record_block(0, 16, 16, 16, true);
    assert_eq!(state.is_cfl_ctx(16, 16, true, true), 2);
}

#[test]
fn uv_cfl_availability_gate_overrides_a_cfl_neighbour() {
    let mut state = TileUvCflState::new(16, 32).unwrap();
    state.record_block(0, 0, 16, 16, true);
    assert_eq!(state.is_cfl_ctx(0, 16, false, false), 0);
}

#[test]
fn uv_cfl_record_block_clips_to_the_grid_and_rejects_empty_dimensions() {
    let mut state = TileUvCflState::new(4, 4).unwrap();
    state.record_block(2, 2, 16, 16, true);
    assert_eq!(state.is_cfl_ctx(3, 3, true, true), 2);
    assert_eq!(state.is_cfl_ctx(2, 3, false, true), 1);
    assert!(TileUvCflState::new(0, 4).is_err());
    assert!(TileUvCflState::new(4, 0).is_err());
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
    let mut state = TileSegmentIdState::new(4, 4).unwrap();
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
    let mut state = TileSegmentIdState::new(4, 4).unwrap();
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
