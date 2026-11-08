// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.4.1 tile-local MI-size state boundary.
//!
//! Feature tracking: `DECODE-TILE-MI-SIZE-STATE-BOUNDARY`.

use std::collections::TryReserveError;
use std::ops::Range;

use super::partition_size::{BlockSize, PartitionSizeError};
use crate::support::reusable_scratch::{recycle_pooled_vec, take_pooled_vec};

const PLANE_COUNT: usize = 2;
const LUMA_PLANE: usize = 0;
const CHROMA_PLANE: usize = 1;
const CLEAR_PARTITION_CONTEXT: u8 = 0;
const PARTITION_CONTEXT_ABOVE: [u8; 29] = [
    63, 63, 62, 62, 62, 60, 60, 60, 56, 56, 56, 48, 48, 48, 32, 32, 32, 0, 0, 63, 60, 62, 56, 60,
    48, 63, 56, 62, 48,
];
const PARTITION_CONTEXT_LEFT: [u8; 29] = [
    63, 62, 63, 62, 60, 62, 60, 56, 60, 56, 48, 56, 48, 32, 48, 32, 0, 32, 0, 60, 63, 56, 62, 48,
    60, 56, 63, 48, 62,
];

/// The § 5.20.4.1 `LeftMiSizes` / `AboveMiSizes` lines of a tile, stored as
/// partition-context bit masks (bit `k` set when the neighbour's log2 extent
/// is at most `k`, in 4x4 units).
///
/// The § 8.3.2 square-split context reads `MiSizes[ 0 ][ r - 1 ][ c ]` and
/// `MiSizes[ 0 ][ r ][ c - 1 ]`. The partition tree decodes the blocks
/// covering one column top to bottom, and the blocks covering one row left
/// to right, so those are the last blocks written to the lines.
///
/// Every read and write stays inside the current superblock row, which each
/// row start clears, so the left lines hold one superblock row indexed by the
/// row modulo `SB_ROW_SPAN`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileMiSizeState {
    origin_row: usize,
    origin_col: usize,
    rows: usize,
    above_len: usize,
    left: [[u8; SB_ROW_SPAN]; PLANE_COUNT],
    /// `[above L | above C]`.
    above: Vec<u8>,
}

/// The tallest superblock in 4x4 units; every superblock height divides it.
const SB_ROW_SPAN: usize = 64;

impl TileMiSizeState {
    pub(crate) fn allocation(
        mi_rows: usize,
        mi_cols: usize,
        sb_size: BlockSize,
    ) -> Result<TileMiSizeStateAllocation, TileMiSizeStateError> {
        if mi_rows == 0 || mi_cols == 0 {
            return Err(TileMiSizeStateError::EmptyDimensions { mi_rows, mi_cols });
        }
        let padded_rows = padded_dimension("rows", mi_rows, sb_size.num_4x4_high()?)?;
        let padded_cols = padded_dimension("cols", mi_cols, sb_size.num_4x4_wide()?)?;
        let entry_count = checked_mul_usize("padded_cols * planes", padded_cols, PLANE_COUNT)?;
        Ok(TileMiSizeStateAllocation {
            padded_rows,
            padded_cols,
            entry_count,
        })
    }

    pub(crate) fn new_for_tile(
        row_range: Range<usize>,
        col_range: Range<usize>,
        sb_size: BlockSize,
    ) -> Result<Self, TileMiSizeStateError> {
        let mi_rows = row_range.end.saturating_sub(row_range.start);
        let mi_cols = col_range.end.saturating_sub(col_range.start);
        let allocation = Self::allocation(mi_rows, mi_cols, sb_size)?;
        let mut above = take_pooled_vec::<u8>(allocation.entry_count);
        above.try_reserve_exact(allocation.entry_count)?;
        above.resize(allocation.entry_count, CLEAR_PARTITION_CONTEXT);
        Ok(Self {
            origin_row: row_range.start,
            origin_col: col_range.start,
            rows: allocation.padded_rows,
            above_len: allocation.padded_cols,
            left: [[CLEAR_PARTITION_CONTEXT; SB_ROW_SPAN]; PLANE_COUNT],
            above,
        })
    }

    const fn above_base(&self, plane: usize) -> usize {
        plane * self.above_len
    }

    #[cfg(test)]
    fn left_at(&self, plane: usize, row: usize) -> u8 {
        self.left[plane][row % SB_ROW_SPAN]
    }

    #[cfg(test)]
    fn above_plane(&self, plane: usize) -> &[u8] {
        let base = self.above_base(plane);
        &self.above[base..base + self.above_len]
    }

    /// Starts a superblock row: clears the left context lines.
    pub(crate) fn enter_sb_row(&mut self, _sb_row: usize) {
        self.left = [[CLEAR_PARTITION_CONTEXT; SB_ROW_SPAN]; PLANE_COUNT];
    }

    /// Calls `visit` with both planes' partition contexts over tile rows
    /// `rows` x columns `cols`, for the encoder facade's rollback.
    pub(crate) fn visit_region(
        &mut self,
        rows: Range<usize>,
        cols: &Range<usize>,
        mut visit: impl FnMut(&mut [u8]),
    ) {
        let local = |range: &Range<usize>, origin: usize, len: usize| {
            range.start.saturating_sub(origin).min(len)..range.end.saturating_sub(origin).min(len)
        };
        let (r, c) = (
            local(&rows, self.origin_row, self.rows),
            local(cols, self.origin_col, self.above_len),
        );
        let start = (self.origin_row + r.start) % SB_ROW_SPAN;
        let r = start..(start + r.len()).min(SB_ROW_SPAN);
        for plane in [LUMA_PLANE, CHROMA_PLANE] {
            let above = self.above_base(plane);
            visit(&mut self.left[plane][r.clone()]);
            visit(&mut self.above[above + c.start..above + c.end]);
        }
    }

    #[inline]
    pub(crate) fn update_luma_block(
        &mut self,
        r: usize,
        c: usize,
        mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        self.update_plane_block(LUMA_PLANE, r, c, mi_size)
    }

    #[inline]
    pub(crate) fn update_chroma_block(
        &mut self,
        chroma_mi_row: usize,
        chroma_mi_col: usize,
        chroma_mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        self.update_plane_block(CHROMA_PLANE, chroma_mi_row, chroma_mi_col, chroma_mi_size)
    }

    /// The § 8.3.2 partition-context entries of the block at tile position
    /// (`r`, `c`): plane `plane` lines at `r`, `r + half_h`, `c` and
    /// `c + half_w`, and the luma lines at `r` and `c`.
    #[inline]
    pub(crate) fn partition_neighbours(
        &self,
        plane: usize,
        r: usize,
        c: usize,
        half_h: usize,
        half_w: usize,
    ) -> Option<PartitionNeighbours> {
        let local_r = r.checked_sub(self.origin_row)?;
        let local_c = c.checked_sub(self.origin_col)?;
        if plane >= PLANE_COUNT
            || local_r + half_h >= self.rows
            || local_c + half_w >= self.above_len
        {
            return None;
        }
        let (left, luma_left) = (&self.left[plane & 1], &self.left[LUMA_PLANE]);
        let row = r % SB_ROW_SPAN;
        let above = self.above.get(self.above_base(plane) + local_c..)?;
        Some(PartitionNeighbours {
            left: [left[row], left[(row + half_h) % SB_ROW_SPAN]],
            above: [*above.first()?, *above.get(half_w)?],
            luma_left: luma_left[row],
            luma_above: *self.above.get(local_c)?,
        })
    }

    #[allow(clippy::inline_always, reason = "measured: two calls per decoded leaf")]
    #[inline(always)]
    fn update_plane_block(
        &mut self,
        plane: usize,
        r: usize,
        c: usize,
        mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        let index = mi_size.index();
        let (Some(local_r), Some(local_c), Some(&left), Some(&above)) = (
            r.checked_sub(self.origin_row),
            c.checked_sub(self.origin_col),
            PARTITION_CONTEXT_LEFT.get(index),
            PARTITION_CONTEXT_ABOVE.get(index),
        ) else {
            return Err(TileMiSizeStateError::BlockOutOfBounds { plane, r, c });
        };
        let rows_end = local_r.saturating_add(mi_size.num_4x4_high()?);
        let cols_end = local_c.saturating_add(mi_size.num_4x4_wide()?);
        if rows_end > self.rows || cols_end > self.above_len {
            return Err(TileMiSizeStateError::BlockOutOfBounds { plane, r, c });
        }
        let row = r % SB_ROW_SPAN;
        if let Some(left_run) = self.left[plane & 1].get_mut(row..row + (rows_end - local_r)) {
            crate::support::fill_mi_run(left_run, left);
        }
        let above_base = self.above_base(plane);
        if let Some(above_run) = self
            .above
            .get_mut(above_base + local_c..above_base + cols_end)
        {
            crate::support::fill_mi_run(above_run, above);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PartitionNeighbours {
    pub(crate) left: [u8; 2],
    pub(crate) above: [u8; 2],
    pub(crate) luma_left: u8,
    pub(crate) luma_above: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileMiSizeStateAllocation {
    padded_rows: usize,
    padded_cols: usize,
    entry_count: usize,
}

impl TileMiSizeStateAllocation {
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn padded_rows(self) -> usize {
        self.padded_rows
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn padded_cols(self) -> usize {
        self.padded_cols
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn entry_count(self) -> usize {
        self.entry_count
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileMiSizeStateError {
    #[error("MI-size state dimensions must be nonzero, got {mi_rows}x{mi_cols}")]
    EmptyDimensions { mi_rows: usize, mi_cols: usize },
    #[error("{operation} overflow: left {left}, right {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("MI-size state allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("MI-size state block-size lookup failed: {0}")]
    Size(#[from] PartitionSizeError),
    #[error("MI-size state plane {plane} block at ({r},{c}) leaves the tile's padded extent")]
    BlockOutOfBounds { plane: usize, r: usize, c: usize },
    #[error(
        "MI-size state padded {axis} dimension overflow: dimension {dimension}, superblock span {sb_span}"
    )]
    PaddedDimensionOverflow {
        axis: &'static str,
        dimension: usize,
        sb_span: usize,
    },
}

fn padded_dimension(
    axis: &'static str,
    dimension: usize,
    sb_span: usize,
) -> Result<usize, TileMiSizeStateError> {
    let sb_pad = sb_span
        .checked_sub(1)
        .ok_or(TileMiSizeStateError::PaddedDimensionOverflow {
            axis,
            dimension,
            sb_span,
        })?;
    let adjusted =
        dimension
            .checked_add(sb_pad)
            .ok_or(TileMiSizeStateError::PaddedDimensionOverflow {
                axis,
                dimension,
                sb_span,
            })?;
    let sb_count = adjusted / sb_span;
    sb_count
        .checked_mul(sb_span)
        .ok_or(TileMiSizeStateError::PaddedDimensionOverflow {
            axis,
            dimension,
            sb_span,
        })
}

fn checked_mul_usize(
    operation: &'static str,
    left: usize,
    right: usize,
) -> Result<usize, TileMiSizeStateError> {
    left.checked_mul(right)
        .ok_or(TileMiSizeStateError::ArithmeticOverflow {
            operation,
            left,
            right,
        })
}

/// Returns the store to the per-thread MI-grid pool; the frontier cursor
/// drops this state at the end of every tile.
impl Drop for TileMiSizeState {
    fn drop(&mut self) {
        recycle_pooled_vec(core::mem::take(&mut self.above));
    }
}

#[cfg(test)]
fn partition_context_above(mi_size_index: usize) -> Option<u8> {
    PARTITION_CONTEXT_ABOVE.get(mi_size_index).copied()
}

#[cfg(test)]
fn partition_context_left(mi_size_index: usize) -> Option<u8> {
    PARTITION_CONTEXT_LEFT.get(mi_size_index).copied()
}

#[cfg(test)]
#[path = "mi_size_state_tests.rs"]
mod tests;
