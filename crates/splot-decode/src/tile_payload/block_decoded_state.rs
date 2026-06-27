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
            // §5.20.2.3 index ranges: x in [-1, (2 * sbSize4) >> subX],
            // y in [-1, sbSize4 >> subY]. Stored with a +1 offset, so the
            // dimensions are the inclusive span length plus the leading -1 cell.
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
            // §5.20.2.3: sbWidth4 = (MiColEnd - c) >> subX,
            // sbHeight4 = (MiRowEnd - r) >> subY (as signed comparisons).
            let sb_width4 = (self.mi_col_end.saturating_sub(c) >> sub_x) as isize;
            let sb_height4 = (self.mi_row_end.saturating_sub(r) >> sub_y) as isize;
            // §5.20.2.3 index ranges (superblock-relative): y in [-1, sbSize4 >> subY],
            // x in [-1, (2 * sbSize4) >> subX].
            let y_max = (self.sb_size4 >> sub_y) as isize;
            let x_max = ((2 * self.sb_size4) >> sub_x) as isize;
            let grid = &mut self.planes[plane];
            for y in -1..=y_max {
                for x in -1..=x_max {
                    // BlockDecoded[plane][y][x] = (y < 0 && x < sbWidth4) ||
                    // (x < 0 && y < sbHeight4); else 0. (The corner [-1][-1] is 1.)
                    let decoded = (y < 0 && x < sb_width4) || (x < 0 && y < sb_height4);
                    if let Some(index) = grid.index(x, y) {
                        grid.cells[index] = decoded;
                    }
                }
            }
            // §5.20.2.3 line 8830 post-loop override:
            // `BlockDecoded[plane][sbSize4 >> subY][-1] = 0`. The main loop's
            // `x < 0 && y < sbHeight4` arm sets this below-left corner to 1 for an
            // interior (non-bottom-edge) superblock; the spec then forces it back to
            // 0. `count_bottom_left_avail` reads `BlockDecoded[plane][y4 + h4 + i][-1]`,
            // so omitting this override would over-count below-left availability.
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
    /// MI_SIZE_LOG2`). For a single full-block transform these equal the block's
    /// plane 4x4 width and height.
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

    // A single 64x64 superblock is 16 luma 4x4 MI units (`sbSize4 == 16`).
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
        // A 128x128 frame (MiCols = MiRowEnd = 32). The top-left superblock at
        // (0, 0): luma sbWidth4 = (32 - 0) >> 0 = 32, sbHeight4 = 32. The above
        // row (y == -1) is decoded for luma columns 0..32 (so the inter-superblock
        // above-right columns 16..31 are decoded); the left column (x == -1) for
        // rows 0..32.
        let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 32, 32).unwrap();
        state.clear_superblock(0, 0);
        // Above row decoded out to column 31 (luma), the §5.20.2.3 cap is
        // sbWidth4 = 32 but the grid only spans x in [-1, 2*16] = [-1, 32].
        assert!(state.flag(0, 0, -1));
        assert!(state.flag(0, 31, -1));
        assert!(state.flag(0, 15, -1));
        // The left column decoded for rows 0..16 (grid y span [-1, 16]).
        assert!(state.flag(0, -1, 0));
        assert!(state.flag(0, -1, 15));
        // Interior cells start undecoded.
        assert!(!state.flag(0, 0, 0));
        assert!(!state.flag(0, 5, 5));
        // §5.20.2.3 sets the top-left corner [-1][-1]: y < 0 && x < sbWidth4 holds
        // for x == -1, so the corner is decoded (1).
        assert!(state.flag(0, -1, -1));
    }

    #[test]
    fn clear_caps_above_row_to_remaining_tile_width() {
        // The rightmost superblock of a 128-wide (MiCols = 32) frame: c = 16.
        // sbWidth4 = (32 - 16) >> 0 = 16, so the above row is decoded only for
        // luma columns 0..16; the above-right columns 16..31 are NOT decoded
        // (there is no in-frame superblock to the upper-right).
        let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 32, 32).unwrap();
        state.clear_superblock(0, 16);
        assert!(state.flag(0, 15, -1));
        assert!(!state.flag(0, 16, -1));
        assert!(!state.flag(0, 31, -1));
    }

    #[test]
    fn split_bottom_left_reads_decoded_top_right_sibling() {
        // 64x64 single superblock (MiCols = MiRowEnd = 16) SPLIT into four 32x32.
        // Decode order TL, TR, BL, BR. After clearing and decoding TL + TR, the
        // bottom-left 32x32 (superblock-relative MI (8, 0), luma 4x4 (x4 = 0,
        // y4 = 8, w4 = 8)) scans BlockDecoded[0][7][8..16): TR occupies MI rows
        // 0..8, cols 8..16, so its bottom row (MI row 7) cols 8..15 are decoded
        // -> num4AboveRight = 8 (the real above-right of the decoded TR sibling).
        let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 16, 16).unwrap();
        state.clear_superblock(0, 0);
        // BL's above-right before TR decodes is not yet available (only the clear
        // marked the above row -1, not the interior row 7).
        assert_eq!(state.count_top_right_avail(0, 0, 8, 8), 0);
        // Decode TL: superblock-relative MI (0, 0), 8x8 luma 4x4 units.
        state.set_block(0, 0, 0, 8, 8);
        // Decode TR: superblock-relative MI (0, 8), 8x8 luma 4x4 units.
        state.set_block(0, 0, 8, 8, 8);
        // Now BL (y4 = 8) reads row 7, columns 8..15 -> all decoded -> 8.
        assert_eq!(state.count_top_right_avail(0, 0, 8, 8), 8);
    }

    #[test]
    fn count_top_right_stops_at_first_undecoded_column() {
        let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 16, 16).unwrap();
        state.clear_superblock(0, 0);
        // Mark only the first two above-right columns of a w4 = 8 sub-block at
        // (x4 = 0, y4 = 8): columns 8 and 9 on row 7.
        state.force_decoded(0, 8, 7);
        state.force_decoded(0, 9, 7);
        assert_eq!(state.count_top_right_avail(0, 0, 8, 8), 2);
    }

    #[test]
    fn count_bottom_left_scans_left_column_below() {
        let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 16, 16).unwrap();
        state.clear_superblock(0, 0);
        // A sub-block at (x4 = 8, y4 = 0, h4 = 8): below-left scans column 7,
        // rows 8..16. Mark rows 8 and 9 decoded.
        state.force_decoded(0, 7, 8);
        state.force_decoded(0, 7, 9);
        assert_eq!(state.count_bottom_left_avail(0, 8, 0, 8), 2);
        // The third row (10) is undecoded -> the count stops at 2.
        assert_eq!(state.count_bottom_left_avail(0, 8, 0, 8), 2);
    }

    #[test]
    fn chroma_plane_uses_subsampled_indices() {
        // 4:2:0 chroma (subX = subY = 1): a 64x64 superblock is 8 chroma 4x4
        // units. A 128x128 frame top-left superblock clear marks the chroma above
        // row decoded for columns 0..((32) >> 1) = 0..16 (capped to the grid span
        // [-1, (2*16) >> 1] = [-1, 16]).
        let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 32, 32).unwrap();
        state.clear_superblock(0, 0);
        assert!(state.flag(1, 0, -1));
        assert!(state.flag(1, 15, -1));
        // Chroma left column decoded for rows 0..8 (grid y span [-1, 8]).
        assert!(state.flag(1, -1, 0));
        assert!(state.flag(1, -1, 7));
    }

    #[test]
    fn clear_overrides_below_left_corner_for_interior_superblock() {
        // §5.20.2.3 line 8830 post-loop override: for an INTERIOR (non-bottom-edge)
        // superblock (mi_row_end > sb_size4, so sbHeight4 > sbSize4) the main loop's
        // `x < 0 && y < sbHeight4` arm sets the below-left corner
        // BlockDecoded[plane][sbSize4][-1] to 1, and the override forces it back to 0.
        // count_bottom_left_avail reads that cell, so it must be 0.
        let mut state = TileBlockDecodedState::new(3, 1, 1, SB_SIZE4, 64, 64).unwrap();
        state.clear_superblock(0, 0);
        let corner_y = SB_SIZE4 as isize;
        // The below-left corner (luma x = -1, y = sbSize4) is forced to 0 despite
        // sbHeight4 (64) > sbSize4 (16).
        assert!(!state.flag(0, -1, corner_y));
        // Sanity: the cell just above it (y = sbSize4 - 1 < sbHeight4) is still the
        // decoded left column.
        assert!(state.flag(0, -1, corner_y - 1));
    }
}
