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

const MAX_PLANES: usize = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileBlockDecodedState {
    subsampling_x: usize,
    subsampling_y: usize,
    num_planes: usize,
    sb_size4: usize,
    mi_col_end: usize,
    mi_row_end: usize,
    planes: [PlaneGrid; MAX_PLANES],
}

#[derive(Debug, Eq, PartialEq, Default)]
struct PlaneGrid {
    width: usize,
    height: usize,
    cells: Vec<bool>,
}

impl Clone for PlaneGrid {
    fn clone(&self) -> Self {
        let mut cells = crate::support::buffer_pool::take::<bool>(self.cells.len());
        cells.extend_from_slice(&self.cells);
        Self {
            width: self.width,
            height: self.height,
            cells,
        }
    }
}

impl TileBlockDecodedState {
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
        if subsampling_x > 1 {
            return Err(TileBlockDecodedStateError::InvalidSubsampling {
                axis: "horizontal",
                value: subsampling_x,
            });
        }
        if subsampling_y > 1 {
            return Err(TileBlockDecodedStateError::InvalidSubsampling {
                axis: "vertical",
                value: subsampling_y,
            });
        }
        let mut planes: [PlaneGrid; MAX_PLANES] = Default::default();
        for (plane, grid) in planes.iter_mut().enumerate().take(num_planes) {
            let (sub_x, sub_y) = plane_subsampling(plane, subsampling_x, subsampling_y);
            let width = (sb_size4
                .checked_mul(2)
                .ok_or(TileBlockDecodedStateError::Overflow)?
                >> sub_x)
                .checked_add(2)
                .ok_or(TileBlockDecodedStateError::Overflow)?;
            let height = (sb_size4 >> sub_y)
                .checked_add(2)
                .ok_or(TileBlockDecodedStateError::Overflow)?;
            let cells_len = width
                .checked_mul(height)
                .ok_or(TileBlockDecodedStateError::Overflow)?;
            let mut cells = crate::support::buffer_pool::take::<bool>(cells_len);
            if cells.capacity() < cells_len {
                cells
                    .try_reserve_exact(cells_len)
                    .map_err(|source| TileBlockDecodedStateError::Allocation { source })?;
            }
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

    pub(crate) const fn num_planes(&self) -> usize {
        self.num_planes
    }

    pub(crate) const fn subsampling(&self, plane: usize) -> (usize, usize) {
        plane_subsampling(plane, self.subsampling_x, self.subsampling_y)
    }

    pub(crate) fn clear_superblock(&mut self, r: usize, c: usize) {
        for plane in 0..self.num_planes {
            let (sub_x, sub_y) = plane_subsampling(plane, self.subsampling_x, self.subsampling_y);
            let sb_width4 = self.mi_col_end.saturating_sub(c) >> sub_x;
            let sb_height4 = self.mi_row_end.saturating_sub(r) >> sub_y;
            let grid = &mut self.planes[plane];
            grid.cells.fill(false);
            let top_len = sb_width4.saturating_add(1).min(grid.width);
            grid.cells[..top_len].fill(true);
            let left_rows = sb_height4.saturating_add(1).min(grid.height);
            for y in 1..left_rows {
                grid.cells[y * grid.width] = true;
            }
            grid.cells[(grid.height - 1) * grid.width] = false;
        }
    }

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
        let base_x = sub_block_mi_col >> sub_x;
        let base_y = sub_block_mi_row >> sub_y;
        let grid = &mut self.planes[plane];
        let start_x = base_x.saturating_add(1);
        let end_x = start_x.saturating_add(step_x4).min(grid.width);
        let end_y = base_y
            .saturating_add(step_y4)
            .saturating_add(1)
            .min(grid.height);
        if start_x < end_x {
            for y in base_y.saturating_add(1)..end_y {
                let row = y * grid.width;
                grid.cells[row + start_x..row + end_x].fill(true);
            }
        }
    }

    pub(crate) fn set_luma_transform(&mut self, x: usize, y: usize, width4: usize, height4: usize) {
        let sb_mask = self.sb_size4.saturating_sub(1);
        self.set_block(0, (y >> 2) & sb_mask, (x >> 2) & sb_mask, width4, height4);
    }

    pub(crate) fn count_top_right_avail(
        &self,
        plane: usize,
        x4: usize,
        y4: usize,
        w4: usize,
    ) -> usize {
        let row = (y4 as isize) - 1;
        self.count_decoded_run(plane, w4, |i| ((x4 + w4 + i) as isize, row))
    }

    pub(crate) fn count_bottom_left_avail(
        &self,
        plane: usize,
        x4: usize,
        y4: usize,
        h4: usize,
    ) -> usize {
        let col = (x4 as isize) - 1;
        self.count_decoded_run(plane, h4, |i| (col, (y4 + h4 + i) as isize))
    }

    fn count_decoded_run(
        &self,
        plane: usize,
        len: usize,
        mut coord: impl FnMut(usize) -> (isize, isize),
    ) -> usize {
        if plane >= self.num_planes {
            return 0;
        }
        let grid = &self.planes[plane];
        let mut decoded = 0;
        for i in 0..len {
            let (x, y) = coord(i);
            if grid.get(x, y) {
                decoded = i + 1;
            } else {
                break;
            }
        }
        decoded
    }

    pub(crate) const fn sb_size4(&self) -> usize {
        self.sb_size4
    }
}

impl PlaneGrid {
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

    fn get(&self, x: isize, y: isize) -> bool {
        self.index(x, y).is_some_and(|index| self.cells[index])
    }
}

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

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileBlockDecodedStateError {
    #[error("BlockDecoded state requires 1..=3 planes, got {num_planes}")]
    InvalidPlanes { num_planes: usize },
    #[error("BlockDecoded state requires a non-empty superblock")]
    EmptySuperblock,
    #[error("BlockDecoded state has invalid {axis} subsampling shift {value}")]
    InvalidSubsampling { axis: &'static str, value: usize },
    #[error("BlockDecoded state dimension overflow")]
    Overflow,
    #[error("BlockDecoded state allocation failed: {source}")]
    Allocation { source: TryReserveError },
}

#[cfg(test)]
#[path = "block_decoded_state_tests.rs"]
mod tests;
