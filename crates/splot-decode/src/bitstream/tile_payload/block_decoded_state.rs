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

const MAX_PLANES: usize = 3;
/// A superblock is at most 64 4x4 cells on each side, so one word holds a row.
const MAX_CELLS: usize = 64;

/// `BlockDecoded` for one superblock: bit `x` of `rows[plane][y]` is cell
/// `(x, y)`. The `y == -1` row and `x == -1` column are not stored: § 5.20.2.3
/// sets them to the leading `above_len` / `left_len` cells, and no block
/// writes them. Cells right of the superblock (`x >= sbSize4`) stay 0 for
/// `y >= 0`, so they need no storage either.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileBlockDecodedState {
    subsampling_x: usize,
    subsampling_y: usize,
    num_planes: usize,
    sb_size4: usize,
    mi_col_end: usize,
    mi_row_end: usize,
    above_len: [usize; MAX_PLANES],
    left_len: [usize; MAX_PLANES],
    plane_rows: [usize; MAX_PLANES],
    rows: [[u64; MAX_CELLS]; MAX_PLANES],
}

impl Default for TileBlockDecodedState {
    fn default() -> Self {
        Self {
            subsampling_x: 0,
            subsampling_y: 0,
            num_planes: 0,
            sb_size4: 0,
            mi_col_end: 0,
            mi_row_end: 0,
            above_len: [0; MAX_PLANES],
            left_len: [0; MAX_PLANES],
            plane_rows: [0; MAX_PLANES],
            rows: [[0; MAX_CELLS]; MAX_PLANES],
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

    /// Lays this state out for another tile.
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
        if sb_size4 > MAX_CELLS {
            return Err(TileBlockDecodedStateError::Overflow);
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
        *self = Self {
            subsampling_x,
            subsampling_y,
            num_planes,
            sb_size4,
            mi_col_end,
            mi_row_end,
            ..Self::default()
        };
        for plane in 0..num_planes {
            self.plane_rows[plane] = sb_size4 >> self.subsampling(plane).1;
        }
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
            let (sub_x, sub_y) = self.subsampling(plane);
            let rows = self.plane_rows[plane];
            self.above_len[plane] = (self.mi_col_end.saturating_sub(c) >> sub_x)
                .min(((2 * self.sb_size4) >> sub_x) + 1);
            self.left_len[plane] = (self.mi_row_end.saturating_sub(r) >> sub_y).min(rows);
            self.rows[plane][..rows].fill(0);
        }
    }

    #[inline]
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
        let (sub_x, sub_y) = self.subsampling(plane);
        let x = sub_block_mi_col >> sub_x;
        let y = sub_block_mi_row >> sub_y;
        let width = step_x4.max(1).min(MAX_CELLS.saturating_sub(x));
        let end_y = (y + step_y4.max(1)).min(self.plane_rows[plane]);
        if width == 0 || y >= end_y {
            return;
        }
        let mask = (u64::MAX >> (MAX_CELLS - width)) << x;
        for row in &mut self.rows[plane][y..end_y] {
            *row |= mask;
        }
    }

    pub(crate) fn set_luma_transform(&mut self, x: usize, y: usize, width4: usize, height4: usize) {
        let sb_mask = self.sb_size4.saturating_sub(1);
        self.set_block(0, (y >> 2) & sb_mask, (x >> 2) & sb_mask, width4, height4);
    }

    /// § 5.20.7.24 `count_top_right_avail` at superblock cell `(x4, y4)`.
    #[inline]
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
        let start = x4 + w4;
        let decoded = match y4.checked_sub(1) {
            None => self.above_len[plane].saturating_sub(start),
            Some(y) if y < self.plane_rows[plane] && start < MAX_CELLS => {
                (self.rows[plane][y] >> start).trailing_ones() as usize
            }
            Some(_) => 0,
        };
        decoded.min(w4)
    }

    /// § 5.20.7.24 `count_bottom_left_avail` at superblock cell `(x4, y4)`.
    #[inline]
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
        let start = y4 + h4;
        let Some(x) = x4.checked_sub(1) else {
            return self.left_len[plane].saturating_sub(start).min(h4);
        };
        let end = (start + h4).min(self.plane_rows[plane]);
        if x >= MAX_CELLS || start >= end {
            return 0;
        }
        self.rows[plane][start..end]
            .iter()
            .take_while(|row| *row >> x & 1 != 0)
            .count()
    }

    pub(crate) const fn sb_size4(&self) -> usize {
        self.sb_size4
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
}

#[cfg(test)]
#[path = "block_decoded_state_tests.rs"]
mod tests;
