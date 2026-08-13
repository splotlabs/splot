// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! What [`super::NeighbourMvGrid`] records for one leaf's motion plane.

#![allow(clippy::unwrap_used)]

use super::{CWP_EQUAL, MotionMode, Mv, warp_sub_mv_at};
use super::{NON_INTER_FLAG_SYNTAX, NeighbourFlagSyntax, NeighbourMotionValues, NeighbourMvGrid};
use crate::prediction::TileGridConstructionError;

#[test]
fn mv_grid_constructor_classifies_geometry_and_allocation() {
    let reversed = std::ops::Range { start: 5, end: 4 };
    let grid = NeighbourMvGrid::new_for_tile(4..8, 8..12).unwrap();
    assert_eq!((grid.origin_row, grid.origin_col), (4, 8));
    assert_eq!((grid.mi_rows, grid.mi_cols), (4, 4));
    assert!(matches!(
        NeighbourMvGrid::new_for_tile(4..4, 8..12),
        Err(TileGridConstructionError::EmptyDimensions)
    ));
    assert!(matches!(
        NeighbourMvGrid::new_for_tile(reversed, 8..12),
        Err(TileGridConstructionError::ReversedDimensions)
    ));
    assert!(matches!(
        NeighbourMvGrid::new_for_tile(0..usize::MAX, 0..2),
        Err(TileGridConstructionError::AreaOverflow)
    ));
    assert!(matches!(
        NeighbourMvGrid::new_for_tile(0..usize::MAX, 0..1),
        Err(TileGridConstructionError::Allocation)
    ));
}

/// A 4x4 leaf at the grid origin whose flag half is already published, since
/// the motion plane is only visible through a cell that carries both halves.
fn leaf_grid(motion_mode: MotionMode) -> NeighbourMvGrid {
    let mut grid = NeighbourMvGrid::new(16, 16).unwrap();
    grid.record_flags(
        0,
        0,
        4,
        4,
        NeighbourFlagSyntax {
            is_inter: true,
            ref_frame0: 0,
            motion_mode,
            ..NON_INTER_FLAG_SYNTAX
        },
    );
    grid
}

/// AV2 § 7.13.3.19 writes `SubMvs` only when `useWarp` is 1 (local warp).
/// Global warp is `useWarp` 2 and writes none, so a `SINGLE_MODE_GLOBALMV`
/// leaf keeps the model its neighbours read while every cell keeps the uniform
/// block MV.
#[test]
fn a_globalmv_single_records_its_mode_without_splatting_sub_mvs() {
    let mv = Mv { row: -20, col: 36 };
    let mut grid = leaf_grid(MotionMode::Simple);

    grid.record_motion(
        0,
        0,
        4,
        4,
        NeighbourMotionValues {
            mv: [mv, Mv::ZERO],
            cwp_weight: CWP_EQUAL,
            stored_warp: None,
            global_mv: [true, false],
            splat_warp: [None, None],
        },
    );

    for (r, c) in [(0, 0), (0, 3), (3, 0), (3, 3)] {
        let cell = grid.get(r, c).unwrap();
        assert_eq!(
            cell.motion.warp_params(),
            None,
            "GLOBALMV does not duplicate frame state at ({r}, {c})"
        );
        assert!(cell.motion.is_global_mv(0));
        assert_eq!(
            cell.motion.sub_mv, mv,
            "no per-cell sub-MV is derived for global warp at ({r}, {c})"
        );
    }
}

/// The same grid does splat when a warp leaf asks it to, so the case above is
/// a deliberate suppression and not a splat that stopped working. The model's
/// linear terms are exaggerated, so two 8x8 units resolve to different sub-MVs
/// through the § 7.13.3.19 rounding.
#[test]
fn a_warp_leaf_still_splats_per_cell_sub_mvs() {
    let model = [131_072, 65_536, 69_632, 4_096, -4_096, 69_632];
    let mv = Mv { row: -20, col: 36 };
    let mut grid = leaf_grid(MotionMode::LocalWarp);

    grid.record_motion(
        0,
        0,
        4,
        4,
        NeighbourMotionValues {
            mv: [mv, Mv::ZERO],
            cwp_weight: CWP_EQUAL,
            stored_warp: Some(model),
            global_mv: [false, false],
            splat_warp: [Some(model), None],
        },
    );

    assert_eq!(
        grid.get(3, 3).unwrap().motion.sub_mv,
        warp_sub_mv_at(model, 0, 0, 3, 3)
    );
    assert_ne!(
        grid.get(0, 0).unwrap().motion.sub_mv,
        grid.get(3, 3).unwrap().motion.sub_mv,
        "a splatted leaf varies its sub-MV across § 7.13.3.19 8x8 units"
    );
}
