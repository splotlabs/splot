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

#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct TileBlockDecodedState {
    subsampling_x: usize,
    subsampling_y: usize,
    num_planes: usize,
    sb_size4: usize,
    mi_col_end: usize,
    mi_row_end: usize,
    planes: [PlaneGrid; MAX_PLANES],
}

/// One bit per cell, bit `x` of `rows[y]`: a bordered superblock row is at
/// most `2 * 64 + 2` cells wide.
#[derive(Debug, Eq, PartialEq, Default)]
struct PlaneGrid {
    width: usize,
    height: usize,
    rows: Vec<BitRow>,
}

const ROW_WORDS: usize = 3;
type BitRow = [u64; ROW_WORDS];

/// The words with bits `start..end` set, `end <= 64 * ROW_WORDS`.
#[inline]
fn run_mask(start: usize, end: usize) -> BitRow {
    core::array::from_fn(|index| {
        let base = index * 64;
        let lo = start.clamp(base, base + 64) - base;
        let hi = end.clamp(base, base + 64) - base;
        let below = |bits: usize| u64::MAX.checked_shl(bits as u32).map_or(u64::MAX, |v| !v);
        below(hi) & !below(lo)
    })
}

#[inline]
fn set_run(row: &mut BitRow, start: usize, end: usize) {
    let mask = run_mask(start, end);
    for (word, bits) in row.iter_mut().zip(mask) {
        *word |= bits;
    }
}

const fn bit(row: &BitRow, x: usize) -> bool {
    row[(x / 64) % ROW_WORDS] >> (x % 64) & 1 != 0
}

/// Set bits of `row` from `start`, at most `len` of them.
#[inline]
fn leading_run(row: &BitRow, start: usize, len: usize) -> usize {
    let mut count = 0;
    while count < len {
        let x = start + count;
        let word = row[(x / 64) % ROW_WORDS] >> (x % 64);
        let ones = (!word).trailing_zeros() as usize;
        count += ones.min(64 - x % 64);
        if ones < 64 - x % 64 {
            break;
        }
    }
    count.min(len)
}

impl Clone for PlaneGrid {
    fn clone(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            rows: self.rows.clone(),
        }
    }

    /// Copies into the cells this grid already holds.
    ///
    /// The decoder keeps a grid per tile for the life of the stream, so the
    /// copy the commit reads is written over the previous tile's rather than
    /// allocating one of its own.
    fn clone_from(&mut self, source: &Self) {
        self.width = source.width;
        self.height = source.height;
        self.rows.clone_from(&source.rows);
    }
}

/// Written by hand so `clone_from` reaches the plane grids' own.
///
/// `#[derive(Clone)]` generates only `clone`, leaving `clone_from` to replace
/// the destination wholesale -- which allocates three plane grids on a
/// destination that already had them.
impl Clone for TileBlockDecodedState {
    fn clone(&self) -> Self {
        Self {
            subsampling_x: self.subsampling_x,
            subsampling_y: self.subsampling_y,
            num_planes: self.num_planes,
            sb_size4: self.sb_size4,
            mi_col_end: self.mi_col_end,
            mi_row_end: self.mi_row_end,
            planes: self.planes.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.subsampling_x = source.subsampling_x;
        self.subsampling_y = source.subsampling_y;
        self.num_planes = source.num_planes;
        self.sb_size4 = source.sb_size4;
        self.mi_col_end = source.mi_col_end;
        self.mi_row_end = source.mi_row_end;
        for (plane, source) in self.planes.iter_mut().zip(&source.planes) {
            plane.clone_from(source);
        }
    }
}

impl TileBlockDecodedState {
    #[cfg(test)]
    pub(crate) fn new(
        num_planes: usize,
        subsampling_x: usize,
        subsampling_y: usize,
        sb_size4: usize,
        mi_col_end: usize,
        mi_row_end: usize,
    ) -> Result<Self, TileBlockDecodedStateError> {
        let mut state = Self::default();
        state.reset(
            num_planes,
            subsampling_x,
            subsampling_y,
            sb_size4,
            mi_col_end,
            mi_row_end,
        )?;
        Ok(state)
    }

    /// Lays this state out for another tile, keeping the plane grids.
    #[inline]
    pub(crate) fn reset(
        &mut self,
        num_planes: usize,
        subsampling_x: usize,
        subsampling_y: usize,
        sb_size4: usize,
        mi_col_end: usize,
        mi_row_end: usize,
    ) -> Result<(), TileBlockDecodedStateError> {
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
        for (plane, grid) in self.planes.iter_mut().enumerate().take(num_planes) {
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
            if width > 64 * ROW_WORDS {
                return Err(TileBlockDecodedStateError::Overflow);
            }
            let mut rows = core::mem::take(&mut grid.rows);
            rows.clear();
            if rows.capacity() < height {
                rows.try_reserve_exact(height)
                    .map_err(|source| TileBlockDecodedStateError::Allocation { source })?;
            }
            rows.resize(height, [0; ROW_WORDS]);
            *grid = PlaneGrid {
                width,
                height,
                rows,
            };
        }
        self.subsampling_x = subsampling_x;
        self.subsampling_y = subsampling_y;
        self.num_planes = num_planes;
        self.sb_size4 = sb_size4;
        self.mi_col_end = mi_col_end;
        self.mi_row_end = mi_row_end;
        Ok(())
    }

    pub(crate) const fn num_planes(&self) -> usize {
        self.num_planes
    }

    pub(crate) const fn subsampling(&self, plane: usize) -> (usize, usize) {
        plane_subsampling(plane, self.subsampling_x, self.subsampling_y)
    }

    #[inline]
    pub(crate) fn clear_superblock(&mut self, r: usize, c: usize) {
        for plane in 0..self.num_planes {
            let (sub_x, sub_y) = plane_subsampling(plane, self.subsampling_x, self.subsampling_y);
            let sb_width4 = self.mi_col_end.saturating_sub(c) >> sub_x;
            let sb_height4 = self.mi_row_end.saturating_sub(r) >> sub_y;
            let grid = &mut self.planes[plane];
            let top_len = sb_width4.saturating_add(1).min(grid.width);
            let left_rows = sb_height4.saturating_add(1).min(grid.height);
            for (y, row) in grid.rows.iter_mut().enumerate() {
                *row = [u64::from(y < left_rows), 0, 0];
            }
            if let Some(top) = grid.rows.first_mut() {
                *top = [0; ROW_WORDS];
                set_run(top, 0, top_len);
            }
            if let Some(bottom) = grid.rows.last_mut() {
                bottom[0] &= !1;
            }
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
            let mask = run_mask(start_x, end_x);
            for row in &mut grid.rows[base_y.saturating_add(1).min(end_y)..end_y] {
                for (word, bits) in row.iter_mut().zip(mask) {
                    *word |= bits;
                }
            }
        }
    }

    pub(crate) fn set_luma_transform(&mut self, x: usize, y: usize, width4: usize, height4: usize) {
        let sb_mask = self.sb_size4.saturating_sub(1);
        self.set_block(0, (y >> 2) & sb_mask, (x >> 2) & sb_mask, width4, height4);
    }

    /// Leading decoded cells of the row above, right of the block (grid row
    /// `y4`, columns from `x4 + w4 + 1` in bordered grid coordinates).
    pub(crate) fn count_top_right_avail(
        &self,
        plane: usize,
        x4: usize,
        y4: usize,
        w4: usize,
    ) -> usize {
        let Some(grid) = self.plane_grid(plane) else {
            return 0;
        };
        let start = x4 + w4 + 1;
        if y4 >= grid.height || start >= grid.width {
            return 0;
        }
        let len = w4.min(grid.width - start);
        leading_run(&grid.rows[y4], start, len)
    }

    /// Leading decoded cells of the column left of the block, below it.
    pub(crate) fn count_bottom_left_avail(
        &self,
        plane: usize,
        x4: usize,
        y4: usize,
        h4: usize,
    ) -> usize {
        let Some(grid) = self.plane_grid(plane) else {
            return 0;
        };
        let start = y4 + h4 + 1;
        if x4 >= grid.width || start >= grid.height {
            return 0;
        }
        let end = (start + h4).min(grid.height);
        grid.rows[start..end]
            .iter()
            .take_while(|row| bit(row, x4))
            .count()
    }

    fn plane_grid(&self, plane: usize) -> Option<&PlaneGrid> {
        if plane >= self.num_planes {
            return None;
        }
        self.planes.get(plane)
    }

    pub(crate) const fn sb_size4(&self) -> usize {
        self.sb_size4
    }
}

#[cfg(test)]
impl PlaneGrid {
    /// The bordered `(row, bit)` of superblock cell `(x, y)`.
    fn index(&self, x: isize, y: isize) -> Option<(usize, usize)> {
        let sx = usize::try_from(x.checked_add(1)?).ok()?;
        let sy = usize::try_from(y.checked_add(1)?).ok()?;
        (sx < self.width && sy < self.height).then_some((sy, sx))
    }

    fn get(&self, x: isize, y: isize) -> bool {
        self.index(x, y)
            .is_some_and(|(row, x)| bit(&self.rows[row], x))
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
