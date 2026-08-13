// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

const SB_SIZE4: usize = 16;

impl TileBlockDecodedState {
    fn flag(&self, plane: usize, x: isize, y: isize) -> bool {
        self.planes[plane].get(x, y)
    }

    fn force_decoded(&mut self, plane: usize, x: isize, y: isize) {
        if let Some(index) = self.planes[plane].index(x, y) {
            self.planes[plane].cells[index] = true;
        }
    }
}

#[test]
fn new_rejects_invalid_geometry() {
    assert!(matches!(
        TileBlockDecodedState::new(0, 1, 1, SB_SIZE4, 16, 16),
        Err(TileBlockDecodedStateError::InvalidPlanes { num_planes: 0 })
    ));
    assert!(matches!(
        TileBlockDecodedState::new(4, 1, 1, SB_SIZE4, 16, 16),
        Err(TileBlockDecodedStateError::InvalidPlanes { num_planes: 4 })
    ));
    assert!(matches!(
        TileBlockDecodedState::new(3, 1, 1, 0, 16, 16),
        Err(TileBlockDecodedStateError::EmptySuperblock)
    ));
    assert!(matches!(
        TileBlockDecodedState::new(3, 1, 1, usize::MAX, 16, 16),
        Err(TileBlockDecodedStateError::Overflow)
    ));
    assert!(matches!(
        TileBlockDecodedState::new(3, usize::BITS as usize, 1, SB_SIZE4, 16, 16),
        Err(TileBlockDecodedStateError::InvalidSubsampling {
            axis: "horizontal",
            value,
        }) if value == usize::BITS as usize
    ));
    assert!(matches!(
        TileBlockDecodedState::new(3, 1, usize::BITS as usize, SB_SIZE4, 16, 16),
        Err(TileBlockDecodedStateError::InvalidSubsampling {
            axis: "vertical",
            value,
        }) if value == usize::BITS as usize
    ));
}

#[test]
fn clear_marks_above_row_and_left_column_within_extent() {
    let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 32, 32).unwrap();
    state.clear_superblock(0, 0);
    assert!(state.flag(0, 0, -1));
    assert!(state.flag(0, 31, -1));
    assert!(state.flag(0, 15, -1));
    assert!(state.flag(0, -1, 0));
    assert!(state.flag(0, -1, 15));
    assert!(!state.flag(0, 0, 0));
    assert!(!state.flag(0, 5, 5));
    assert!(state.flag(0, -1, -1));
}

#[test]
fn clear_caps_above_row_to_remaining_tile_width() {
    let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 32, 32).unwrap();
    state.clear_superblock(0, 16);
    assert!(state.flag(0, 15, -1));
    assert!(!state.flag(0, 16, -1));
    assert!(!state.flag(0, 31, -1));
}

#[test]
fn clear_caps_left_column_and_resets_stale_interior_flags() {
    let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 32, 20).unwrap();
    state.force_decoded(0, 5, 5);
    state.clear_superblock(16, 0);
    assert!(state.flag(0, -1, 3));
    assert!(!state.flag(0, -1, 4));
    assert!(!state.flag(0, 5, 5));
}

#[test]
fn split_bottom_left_reads_decoded_top_right_sibling() {
    let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 16, 16).unwrap();
    state.clear_superblock(0, 0);
    assert_eq!(state.count_top_right_avail(0, 0, 8, 8), 0);
    state.set_block(0, 0, 0, 8, 8);
    state.set_luma_transform(32, 0, 8, 8);
    assert_eq!(state.count_top_right_avail(0, 0, 8, 8), 8);
}

#[test]
fn count_top_right_stops_at_first_undecoded_column() {
    let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 16, 16).unwrap();
    state.clear_superblock(0, 0);
    state.force_decoded(0, 8, 7);
    state.force_decoded(0, 9, 7);
    assert_eq!(state.count_top_right_avail(0, 0, 8, 8), 2);
}

#[test]
fn set_block_marks_thin_subsampled_chroma_transform_unit() {
    let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 32, 32).unwrap();
    state.clear_superblock(0, 0);

    state.set_block(1, 9, 12, 2, 0);

    assert!(state.flag(1, 6, 4));
    assert_eq!(state.count_top_right_avail(1, 5, 5, 1), 1);
}

#[test]
fn count_bottom_left_scans_left_column_below() {
    let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 16, 16).unwrap();
    state.clear_superblock(0, 0);
    state.force_decoded(0, 7, 8);
    state.force_decoded(0, 7, 9);
    assert_eq!(state.count_bottom_left_avail(0, 8, 0, 8), 2);
    assert_eq!(state.count_bottom_left_avail(0, 8, 0, 8), 2);
}

#[test]
fn chroma_plane_uses_subsampled_indices() {
    let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 32, 32).unwrap();
    state.clear_superblock(0, 0);
    assert!(state.flag(1, 0, -1));
    assert!(state.flag(1, 15, -1));
    assert!(state.flag(1, -1, 0));
    assert!(state.flag(1, -1, 7));
}

#[test]
fn clear_overrides_below_left_corner_for_interior_superblock() {
    let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 64, 64).unwrap();
    state.clear_superblock(0, 0);
    let corner_y = SB_SIZE4 as isize;
    assert!(!state.flag(0, -1, corner_y));
    assert!(state.flag(0, -1, corner_y - 1));
}
