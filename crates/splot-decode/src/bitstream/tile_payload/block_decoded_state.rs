// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.2.3 per-block `BlockDecoded` flag state.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-BLOCK-DECODED`.
//!
//! `BlockDecoded[plane][y][x]` stores one boolean per 4x4 sample block per plane,
//! where a `1` means the corresponding 4x4 block has already been decoded
//! (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md` line 6405). The array
//! is **superblock-relative**: every index is measured from the current
//! superblock origin, so `y == -1` / `x == -1` are the top / left edge and the
//! array is re-initialized by § 5.20.2.3 `clear_block_decoded_flags` at the start
//! of every superblock (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-3`).
//!
//! The general intra § 7.13.2.1 edge derivation reads the above-right
//! (`num4AboveRight`) and below-left (`num4BelowLeft`) availability via § 5.20.7.25
//! `count_top_right_avail` / `count_bottom_left_avail`
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md`, lines 15173 / 15185) over this
//! state; the per-block update (`BlockDecoded[plane][(subBlockMiRow >> subY) + i]
//! [(subBlockMiCol >> subX) + j] = 1`, line 15113) is applied after each
//! transform block reconstructs so a later sub-block reads the correct sentinel.

use std::collections::TryReserveError;

/// AV2 `NumPlanes` upper bound for 4:2:0 / 4:2:2 / 4:4:4 (Y, U, V).
const MAX_PLANES: usize = 3;

/// Per-plane superblock-relative AV2 § 5.20.2.3 `BlockDecoded` grid.
///
/// Each plane stores a dense boolean grid covering the § 5.20.2.3 index range
/// `y in [-1, sbSize4 >> subY]` and `x in [-1, (2 * sbSize4) >> subX]` (the
/// trailing `>> subX/subY` accounts for 4:2:0 / 4:2:2 chroma subsampling). The
/// `-1` edge rows/columns are stored at offset `+1`, so an array index is
/// `(y + 1) * width + (x + 1)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileBlockDecodedState {
    /// Validated subsampling (`subX == 1` for 4:2:0 / 4:2:2 chroma horizontal).
    subsampling_x: usize,
    /// Validated subsampling (`subY == 1` for 4:2:0 chroma vertical).
    subsampling_y: usize,
    /// AV2 `NumPlanes` actually present (1 monochrome, 3 otherwise).
    num_planes: usize,
    /// Superblock width in luma 4x4 MI units (`Num_4x4_Blocks_Wide[SbSize]`).
    sb_size4: usize,
    /// Tile MI column end (`MiColEnd`, exclusive) for the active tile.
    mi_col_end: usize,
    /// Tile MI row end (`MiRowEnd`, exclusive) for the active tile.
    mi_row_end: usize,
    /// Per-plane grid dimensions and storage; index `[plane]`.
    planes: [PlaneGrid; MAX_PLANES],
}

/// Per-plane dense `BlockDecoded` grid storage.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
struct PlaneGrid {
    /// Grid width in stored columns (`((2 * sbSize4) >> subX) + 2`, the `x`
    /// range `[-1, (2 * sbSize4) >> subX]` plus the `+1` offset).
    width: usize,
    /// Grid height in stored rows (`(sbSize4 >> subY) + 2`).
    height: usize,
    /// Row-major boolean cells (`width * height`).
    cells: Vec<bool>,
}

impl TileBlockDecodedState {
    /// Allocates a `BlockDecoded` state for the given superblock geometry and
    /// chroma subsampling. `sb_size4` is `Num_4x4_Blocks_Wide[SbSize]` (the
    /// superblock side in luma 4x4 MI units); `mi_col_end` / `mi_row_end` are the
    /// active tile's `MiColEnd` / `MiRowEnd` (exclusive).
    pub(crate) fn new(
        num_planes: usize,
        subsampling_x: usize,
        subsampling_y: usize,
        sb_size4: usize,
        mi_col_end: usize,
        mi_row_end: usize,
    ) -> Result<Self, TileBlockDecodedStateError> {
        if num_planes == 0 || num_planes > MAX_PLANES {
            return Err(TileBlockDecodedStateError::InvalidPlanes { num_planes });
        }
        if sb_size4 == 0 {
            return Err(TileBlockDecodedStateError::EmptySuperblock);
        }
        let mut planes: [PlaneGrid; MAX_PLANES] = Default::default();
        for (plane, grid) in planes.iter_mut().enumerate().take(num_planes) {
            let (sub_x, sub_y) = plane_subsampling(plane, subsampling_x, subsampling_y);
            let width = ((2 * sb_size4) >> sub_x)
                .checked_add(2)
                .ok_or(TileBlockDecodedStateError::Overflow)?;
            let height = (sb_size4 >> sub_y)
                .checked_add(2)
                .ok_or(TileBlockDecodedStateError::Overflow)?;
            let cells_len = width
                .checked_mul(height)
                .ok_or(TileBlockDecodedStateError::Overflow)?;
            let mut cells = Vec::new();
            cells
                .try_reserve_exact(cells_len)
                .map_err(|source| TileBlockDecodedStateError::Allocation { source })?;
            cells.resize(cells_len, false);
            *grid = PlaneGrid {
                width,
                height,
                cells,
            };
        }
        Ok(Self {
            subsampling_x,
            subsampling_y,
            num_planes,
            sb_size4,
            mi_col_end,
            mi_row_end,
            planes,
        })
    }

    /// AV2 § 5.20.2.3 `clear_block_decoded_flags(r, c, sbSize4)`: re-initializes
    /// `BlockDecoded` for the superblock rooted at luma MI position (`r`, `c`).
    ///
    /// For each plane the above row (`y == -1`) is set decoded for plane columns
    /// `x < sbWidth4 = (MiColEnd - c) >> subX` and the left column (`x == -1`) is
    /// set decoded for plane rows `y < sbHeight4 = (MiRowEnd - r) >> subY`; every
    /// other cell is cleared. The spec's post-loop override
    /// `BlockDecoded[plane][sbSize4 >> subY][-1] = 0` (§5.20.2.3 line 8830) is then
    /// applied explicitly: for an interior (non-bottom-edge) superblock the main
    /// loop's `x < 0 && y < sbHeight4` arm sets that below-left corner to 1, and the
    /// override forces it back to 0 (`count_bottom_left_avail` reads that cell)
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-3`).
    pub(crate) fn clear_superblock(&mut self, r: usize, c: usize) {
        for plane in 0..self.num_planes {
            let (sub_x, sub_y) = plane_subsampling(plane, self.subsampling_x, self.subsampling_y);
            let sb_width4 = (self.mi_col_end.saturating_sub(c) >> sub_x) as isize;
            let sb_height4 = (self.mi_row_end.saturating_sub(r) >> sub_y) as isize;
            let y_max = (self.sb_size4 >> sub_y) as isize;
            let x_max = ((2 * self.sb_size4) >> sub_x) as isize;
            let grid = &mut self.planes[plane];
            for y in -1..=y_max {
                for x in -1..=x_max {
                    let decoded = (y < 0 && x < sb_width4) || (x < 0 && y < sb_height4);
                    if let Some(index) = grid.index(x, y) {
                        grid.cells[index] = decoded;
                    }
                }
            }
            if let Some(index) = grid.index(-1, y_max) {
                grid.cells[index] = false;
            }
        }
    }

    /// AV2 § 5.20.4 `BlockDecoded[plane][(subBlockMiRow >> subY) + i]
    /// [(subBlockMiCol >> subX) + j] = 1`: marks every plane 4x4 unit of a decoded
    /// transform block (`docs/spec/av2/1.0.0/05-syntax-structures.md` line 15113).
    ///
    /// `sub_block_mi_row` and `sub_block_mi_col` are the block's
    /// **superblock-relative** luma MI position (`row & sbMask`, `col & sbMask`);
    /// `step_x4` and `step_y4` are the transform-block width and height in plane
    /// 4x4 units (`Tx_Width[txSz] >> MI_SIZE_LOG2`, `Tx_Height[txSz] >>
    /// MI_SIZE_LOG2`), clamped to the minimum one plane 4x4 unit when chroma
    /// subsampling maps a thin luma block to a 4-sample chroma transform. For a
    /// single full-block transform these equal the block's plane 4x4 width and
    /// height.
    pub(crate) fn set_block(
        &mut self,
        plane: usize,
        sub_block_mi_row: usize,
        sub_block_mi_col: usize,
        step_x4: usize,
        step_y4: usize,
    ) {
        if plane >= self.num_planes {
            return;
        }
        let step_x4 = step_x4.max(1);
        let step_y4 = step_y4.max(1);
        let (sub_x, sub_y) = plane_subsampling(plane, self.subsampling_x, self.subsampling_y);
        let base_x = (sub_block_mi_col >> sub_x) as isize;
        let base_y = (sub_block_mi_row >> sub_y) as isize;
        let grid = &mut self.planes[plane];
        for i in 0..step_y4 {
            for j in 0..step_x4 {
                let x = base_x.saturating_add(j as isize);
                let y = base_y.saturating_add(i as isize);
                if let Some(index) = grid.index(x, y) {
                    grid.cells[index] = true;
                }
            }
        }
    }

    /// AV2 § 5.20.7.25 `count_top_right_avail(plane, x4, y4, w4)`: counts how many
    /// 4x4 columns above and to the right of a sub-block have already been decoded
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md` line 15173). `x4` / `y4` are
    /// the **superblock-relative** sub-block position in plane 4x4 units
    /// (`subBlockMiCol >> subX`, `subBlockMiRow >> subY`); `w4` is the block width
    /// in plane 4x4 units. Scans `BlockDecoded[plane][y4 - 1][x4 + w4 + i]` for
    /// `i in 0..w4`, stopping at the first undecoded column.
    pub(crate) fn count_top_right_avail(
        &self,
        plane: usize,
        x4: usize,
        y4: usize,
        w4: usize,
    ) -> usize {
        if plane >= self.num_planes {
            return 0;
        }
        let grid = &self.planes[plane];
        let row = (y4 as isize) - 1;
        let mut num_top_right = 0;
        for i in 0..w4 {
            let col = (x4 + w4 + i) as isize;
            if grid.get(col, row) {
                num_top_right = i + 1;
            } else {
                break;
            }
        }
        num_top_right
    }

    /// AV2 § 5.20.7.25 `count_bottom_left_avail(plane, x4, y4, h4)`: counts how
    /// many 4x4 rows below and to the left of a sub-block have already been
    /// decoded (`docs/spec/av2/1.0.0/05-syntax-structures.md` line 15185). Scans
    /// `BlockDecoded[plane][y4 + h4 + i][x4 - 1]` for `i in 0..h4`, stopping at the
    /// first undecoded row.
    pub(crate) fn count_bottom_left_avail(
        &self,
        plane: usize,
        x4: usize,
        y4: usize,
        h4: usize,
    ) -> usize {
        if plane >= self.num_planes {
            return 0;
        }
        let grid = &self.planes[plane];
        let col = (x4 as isize) - 1;
        let mut num_bottom_left = 0;
        for i in 0..h4 {
            let row = (y4 + h4 + i) as isize;
            if grid.get(col, row) {
                num_bottom_left = i + 1;
            } else {
                break;
            }
        }
        num_bottom_left
    }

    /// Superblock side in luma 4x4 MI units, for the caller's `sbMask` derivation.
    pub(crate) const fn sb_size4(&self) -> usize {
        self.sb_size4
    }

    /// Test-only: reads the superblock-relative flag at (`x`, `y`) for `plane`.
    #[cfg(test)]
    fn flag(&self, plane: usize, x: isize, y: isize) -> bool {
        self.planes[plane].get(x, y)
    }

    /// Test-only: forces the superblock-relative flag at (`x`, `y`) for `plane`.
    #[cfg(test)]
    fn force_decoded(&mut self, plane: usize, x: isize, y: isize) {
        if let Some(index) = self.planes[plane].index(x, y) {
            self.planes[plane].cells[index] = true;
        }
    }
}

impl PlaneGrid {
    /// Row-major index for superblock-relative (`x`, `y`) with the `-1` edge
    /// stored at offset `+1`, or `None` when out of the stored span.
    fn index(&self, x: isize, y: isize) -> Option<usize> {
        let sx = x.checked_add(1)?;
        let sy = y.checked_add(1)?;
        if sx < 0 || sy < 0 {
            return None;
        }
        let sx = sx as usize;
        let sy = sy as usize;
        if sx >= self.width || sy >= self.height {
            return None;
        }
        sy.checked_mul(self.width)?.checked_add(sx)
    }

    /// Reads the flag at superblock-relative (`x`, `y`); out-of-span reads are
    /// `false` (undecoded), matching the spec's `BlockDecoded` default.
    fn get(&self, x: isize, y: isize) -> bool {
        self.index(x, y).is_some_and(|index| self.cells[index])
    }
}

/// Per-plane subsampling: plane 0 (luma) is never subsampled; chroma planes use
/// the frame `SubsamplingX` / `SubsamplingY`.
const fn plane_subsampling(
    plane: usize,
    subsampling_x: usize,
    subsampling_y: usize,
) -> (usize, usize) {
    if plane == 0 {
        (0, 0)
    } else {
        (subsampling_x, subsampling_y)
    }
}

/// Error raised while building or sizing the `BlockDecoded` state.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TileBlockDecodedStateError {
    /// `NumPlanes` was zero or above the 4:4:4 maximum.
    #[error("BlockDecoded state requires 1..=3 planes, got {num_planes}")]
    InvalidPlanes {
        /// Requested plane count.
        num_planes: usize,
    },
    /// The superblock side was zero.
    #[error("BlockDecoded state requires a non-empty superblock")]
    EmptySuperblock,
    /// A dimension arithmetic computation overflowed `usize`.
    #[error("BlockDecoded state dimension overflow")]
    Overflow,
    /// The grid allocation failed.
    #[error("BlockDecoded state allocation failed: {source}")]
    Allocation {
        /// Underlying reservation error.
        source: TryReserveError,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const SB_SIZE4: usize = 16;

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
    fn split_bottom_left_reads_decoded_top_right_sibling() {
        let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 16, 16).unwrap();
        state.clear_superblock(0, 0);
        assert_eq!(state.count_top_right_avail(0, 0, 8, 8), 0);
        state.set_block(0, 0, 0, 8, 8);
        state.set_block(0, 0, 8, 8, 8);
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
}
