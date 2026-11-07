// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.4.1 tile-local MI-size state boundary.
//!
//! Feature tracking: `DECODE-TILE-MI-SIZE-STATE-BOUNDARY`.

use std::collections::TryReserveError;
use std::ops::Range;

use super::partition_size::{BlockSize, PartitionSizeError};
use super::partition_traversal::TilePartitionContextState;
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileMiSizeState {
    origin_row: usize,
    origin_col: usize,
    left_len: usize,
    above_len: usize,
    /// `[left L | left C | above L | above C]`.
    storage: Vec<u8>,
}

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
        let lines = checked_add_usize("padded_rows + padded_cols", padded_rows, padded_cols)?;
        let entry_count = checked_mul_usize("lines * planes", lines, PLANE_COUNT)?;
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
        let mut storage = take_pooled_vec::<u8>(allocation.entry_count);
        storage.try_reserve_exact(allocation.entry_count)?;
        storage.resize(allocation.entry_count, CLEAR_PARTITION_CONTEXT);
        Ok(Self {
            origin_row: row_range.start,
            origin_col: col_range.start,
            left_len: allocation.padded_rows,
            above_len: allocation.padded_cols,
            storage,
        })
    }

    const fn left_base(&self, plane: usize) -> usize {
        plane * self.left_len
    }

    const fn above_base(&self, plane: usize) -> usize {
        PLANE_COUNT * self.left_len + plane * self.above_len
    }

    fn left_plane(&self, plane: usize) -> &[u8] {
        let base = self.left_base(plane);
        &self.storage[base..base + self.left_len]
    }

    fn above_plane(&self, plane: usize) -> &[u8] {
        let base = self.above_base(plane);
        &self.storage[base..base + self.above_len]
    }

    /// Starts a superblock row: clears the left context lines.
    pub(crate) fn enter_sb_row(&mut self, _sb_row: usize) {
        self.storage[..PLANE_COUNT * self.left_len].fill(CLEAR_PARTITION_CONTEXT);
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
            local(&rows, self.origin_row, self.left_len),
            local(cols, self.origin_col, self.above_len),
        );
        for plane in [LUMA_PLANE, CHROMA_PLANE] {
            let (left, above) = (self.left_base(plane), self.above_base(plane));
            visit(&mut self.storage[left + r.start..left + r.end]);
            visit(&mut self.storage[above + c.start..above + c.end]);
        }
    }

    pub(crate) fn update_luma_block(
        &mut self,
        r: usize,
        c: usize,
        mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        self.update_plane_block(LUMA_PLANE, r, c, mi_size)
    }

    pub(crate) fn update_chroma_block(
        &mut self,
        chroma_mi_row: usize,
        chroma_mi_col: usize,
        chroma_mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        self.update_plane_block(CHROMA_PLANE, chroma_mi_row, chroma_mi_col, chroma_mi_size)
    }

    pub(crate) fn context_state(&self) -> TilePartitionContextState<'_> {
        TilePartitionContextState::new_at(
            [self.left_plane(LUMA_PLANE), self.left_plane(CHROMA_PLANE)],
            [self.above_plane(LUMA_PLANE), self.above_plane(CHROMA_PLANE)],
            self.origin_row,
            self.origin_col,
        )
    }

    #[inline]
    fn update_plane_block(
        &mut self,
        plane: usize,
        r: usize,
        c: usize,
        mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        let index = mi_size.index();
        let out_of_bounds = TileMiSizeStateError::BlockOutOfBounds { plane, r, c };
        let (Some(local_r), Some(local_c), Some(&left), Some(&above)) = (
            r.checked_sub(self.origin_row),
            c.checked_sub(self.origin_col),
            PARTITION_CONTEXT_LEFT.get(index),
            PARTITION_CONTEXT_ABOVE.get(index),
        ) else {
            return Err(out_of_bounds);
        };
        let rows = local_r..local_r.saturating_add(mi_size.num_4x4_high()?);
        let cols = local_c..local_c.saturating_add(mi_size.num_4x4_wide()?);
        if rows.end > self.left_len || cols.end > self.above_len {
            return Err(out_of_bounds);
        }
        let (left_base, above_base) = (self.left_base(plane), self.above_base(plane));
        crate::support::fill_mi_run(
            &mut self.storage[left_base + rows.start..left_base + rows.end],
            left,
        );
        crate::support::fill_mi_run(
            &mut self.storage[above_base + cols.start..above_base + cols.end],
            above,
        );
        Ok(())
    }
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

fn checked_add_usize(
    operation: &'static str,
    left: usize,
    right: usize,
) -> Result<usize, TileMiSizeStateError> {
    left.checked_add(right)
        .ok_or(TileMiSizeStateError::ArithmeticOverflow {
            operation,
            left,
            right,
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
        recycle_pooled_vec(core::mem::take(&mut self.storage));
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
