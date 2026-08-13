// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::prediction::TileGridConstructionError;

#[test]
fn smooth_grid_constructor_classifies_invalid_geometry_and_allocation() {
    let reversed = std::ops::Range { start: 5, end: 4 };
    assert!(matches!(
        TileYSmoothGrid::new_for_tile(4..4, 8..12),
        Err(TileGridConstructionError::EmptyDimensions)
    ));
    assert!(matches!(
        TileYSmoothGrid::new_for_tile(reversed, 8..12),
        Err(TileGridConstructionError::ReversedDimensions)
    ));
    assert!(matches!(
        TileYSmoothGrid::new_for_tile(0..usize::MAX, 0..2),
        Err(TileGridConstructionError::AreaOverflow)
    ));
    assert!(matches!(
        TileYSmoothGrid::new_for_tile(0..usize::MAX, 0..1),
        Err(TileGridConstructionError::Allocation)
    ));
}

#[test]
fn grid_records_and_reads_cells() {
    let mut grid = TileYSmoothGrid::new_for_tile(0..4, 0..4).unwrap();
    grid.record(1, 1, 2, 2, true);
    assert!(grid.at(1, 1));
    assert!(grid.at(2, 2));
    assert!(!grid.at(0, 0));
    assert!(!grid.at(-1, 1));
    assert!(!grid.at(1, 4));
}

#[test]
fn tile_local_grid_translates_absolute_coordinates() {
    let mut grid = TileYSmoothGrid::new_for_tile(4..8, 8..12).unwrap();
    grid.record(5, 9, 2, 2, true);
    assert!(grid.at(9, 5));
    assert!(grid.at(10, 6));
    assert!(!grid.at(8, 4));
    assert!(!grid.at(7, 5));
    assert_eq!(grid.block_smoothness(10, 6), (true, true));
}

#[test]
fn one_sided_spec_ors_smoothness_without_ibp() {
    let spec = one_sided_read_edge_spec(false, true, 45, false);
    assert!(spec.above);
    assert!(spec.filter_type, "zone-1 without IBP ORs above|left");
    assert_eq!(spec.angle_delta, -45);
    assert!(spec.need_far);
    assert!(!spec.corner_applies, "one-sided zone-1 lacks needLeft");
}

#[test]
fn one_sided_spec_keeps_per_edge_smoothness_under_ibp() {
    let spec = one_sided_read_edge_spec(false, true, 45, true);
    assert!(!spec.filter_type, "applyIbp keeps the per-edge above type");
    assert!(spec.corner_applies, "applyIbp forces needAbove && needLeft");
    let secondary = ibp_secondary_edge_spec(false, true, 45);
    assert!(!secondary.above);
    assert!(secondary.filter_type, "secondary reads the left type");
    assert_eq!(secondary.angle_delta, 45 - 180 + 180);
    assert!(secondary.need_far, "zone-1 secondary needs the bottom span");
}

#[test]
fn zone3_secondary_reads_above_with_wrapped_angle() {
    let secondary = ibp_secondary_edge_spec(true, false, 203);
    assert!(secondary.above);
    assert!(secondary.filter_type);
    assert_eq!(secondary.angle_delta, 203 - 90 - 180);
    assert!(secondary.need_far, "pAngle > 180 needs the right span");
}
