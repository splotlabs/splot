// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

const WHOLE_TILE: usize = crate::tile::SbRowWindow::WHOLE_TILE_SB_H4;

const SB_N4: usize = 16;

fn state(rows: usize, cols: usize, sb_size4: usize) -> TileIntraFactsState {
    TileIntraFactsState::new_for_tile(0..rows, 0..cols, sb_size4).unwrap()
}

fn record(state: &mut TileIntraFactsState, (r, c): (usize, usize), n4: usize, facts: IntraFacts) {
    state.record_leaf((r, c), (n4, n4), facts);
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
fn y_mode_facts_record_clip_and_read_back_intrabc_as_dc() {
    let mut state = state(4, 4, WHOLE_TILE);
    let facts = IntraFacts::luma(
        IntraJointMode::DC,
        IntraYMode::Dc,
        -3,
        0,
        MrlSelection::Disabled,
    );
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
    assert!(!state.uses_fsc_at(2, 2));
    record(
        &mut state,
        (0, 0),
        1,
        IntraFacts::luma(
            IntraJointMode::DC,
            IntraYMode::Dc,
            0,
            1,
            MrlSelection::Disabled,
        ),
    );
    assert!(state.uses_fsc_at(0, 0));
    assert_eq!(
        state.y_mode_facts_at(0, 2),
        Some(TileIntraYModeFacts {
            y_mode: IntraYMode::Dc,
            angle_delta_y: 0,
        })
    );
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
