// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

fn recorded_trajectory(
    state: &TrajectoryState,
    reference: usize,
    at: Position,
    phase: usize,
) -> Option<Position> {
    let index = temporal_grid_index(state.width8, state.height8, at.0, at.1)?;
    let positions = state.positions.get(reference)?.get(index)?;
    if positions.mask & (1 << phase) == 0 {
        return None;
    }
    positions.phases.get(phase)?.unpack()
}

#[test]
fn forward_intersection_samples_the_clipped_positive_boundary() {
    let mut state = TrajectoryState::new((2, 80), 2, 2, 16).unwrap();
    state.whole_band().unwrap().observe_projection(
        0,
        None,
        None,
        0,
        2,
        Mv { row: 0, col: 1984 },
        -1,
        31,
        false,
    );

    state
        .whole_band()
        .unwrap()
        .check_intersection(0, Some(1), 0, 2, Mv { row: 0, col: 1984 });

    assert_eq!(state.fields[1].cell(0, 0), Some(Mv { row: 0, col: 2047 }));
    assert_eq!(recorded_trajectory(&state, 1, (0, 30), 0), Some((0, 0)));
    assert_eq!(recorded_trajectory(&state, 1, (0, 32), 0), None);
}

#[test]
fn reverse_intersection_samples_the_clipped_positive_boundary() {
    let mut state = TrajectoryState::new((2, 160), 3, 2, 16).unwrap();
    state.whole_band().unwrap().observe_projection(
        2,
        Some(1),
        None,
        0,
        34,
        Mv { row: 0, col: -1088 },
        19,
        17,
        false,
    );

    state
        .whole_band()
        .unwrap()
        .check_intersection(0, Some(1), 0, 46, Mv { row: 0, col: -1920 });

    assert_eq!(state.fields[0].cell(0, 16), Some(Mv { row: 0, col: 2047 }));
    assert_eq!(recorded_trajectory(&state, 0, (0, 46), 1), Some((0, 16)));
    assert_eq!(recorded_trajectory(&state, 0, (0, 48), 1), None);
}

#[test]
fn negative_boundary_positions_differ_but_both_exceed_the_unit_bounds() {
    let mut state = TrajectoryState::new((2, 160), 2, 1, 8).unwrap();
    state.whole_band().unwrap().observe_projection(
        0,
        None,
        None,
        0,
        34,
        Mv { row: 0, col: -1984 },
        -1,
        31,
        false,
    );

    state
        .whole_band()
        .unwrap()
        .check_intersection(0, Some(1), 0, 34, Mv { row: 0, col: -1984 });

    assert_eq!(state.fields[1].cell(0, 35), Some(Mv { row: 0, col: -2047 }));
    assert_eq!(recorded_trajectory(&state, 1, (0, 3), 0), None);
    assert_eq!(recorded_trajectory(&state, 1, (0, 4), 0), None);
    let band = state.whole_band().unwrap();
    assert_eq!(
        band.sampled_position(0, 35, Mv { row: 0, col: -2048 }),
        Some((0, 3))
    );
    assert_eq!(
        band.sampled_position(0, 35, Mv { row: 0, col: -2047 }),
        Some((0, 4))
    );
}
