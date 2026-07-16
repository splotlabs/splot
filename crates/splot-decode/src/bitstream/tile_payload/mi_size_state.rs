// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.4.1 tile-local MI-size state boundary.
//!
//! Feature tracking: `DECODE-TILE-MI-SIZE-STATE-BOUNDARY`.

use std::collections::TryReserveError;
use std::ops::Range;

use super::partition_size::{BlockSize, PartitionSizeError};
use super::partition_traversal::TilePartitionContextState;

const BLOCK_256X256_INDEX: usize = 18;
const PLANE_COUNT: usize = 2;
const LUMA_PLANE: usize = 0;
const CHROMA_PLANE: usize = 1;
const CLEAR_PARTITION_CONTEXT: usize = 0;
const PARTITION_CONTEXT_ABOVE: [usize; 29] = [
    63, 63, 62, 62, 62, 60, 60, 60, 56, 56, 56, 48, 48, 48, 32, 32, 32, 0, 0, 63, 60, 62, 56, 60,
    48, 63, 56, 62, 48,
];
const PARTITION_CONTEXT_LEFT: [usize; 29] = [
    63, 62, 63, 62, 60, 62, 60, 56, 60, 56, 48, 56, 48, 32, 48, 32, 0, 32, 0, 60, 63, 56, 62, 48,
    60, 56, 63, 48, 62,
];

type PlaneBuffers = [Vec<usize>; PLANE_COUNT];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileMiSizeState {
    origin_row: usize,
    origin_col: usize,
    mi_rows: usize,
    mi_cols: usize,
    mi_sizes: PlaneBuffers,
    mi_size_stride: usize,
    left_mi_sizes: PlaneBuffers,
    above_mi_sizes: PlaneBuffers,
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
        let padded_grid_cells =
            checked_mul_usize("padded_mi_rows * padded_mi_cols", padded_rows, padded_cols)?;
        let plane_entries = checked_add_usize(
            "padded_grid_cells + padded_rows",
            padded_grid_cells,
            padded_rows,
        )?;
        let plane_entries =
            checked_add_usize("plane_entries + padded_cols", plane_entries, padded_cols)?;
        let entry_count = checked_mul_usize("plane_entries * planes", plane_entries, PLANE_COUNT)?;
        Ok(TileMiSizeStateAllocation {
            padded_rows,
            padded_cols,
            padded_grid_cells,
            entry_count,
        })
    }

    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size: BlockSize,
    ) -> Result<Self, TileMiSizeStateError> {
        Self::new_for_tile(0..mi_rows, 0..mi_cols, sb_size)
    }

    pub(crate) fn new_for_tile(
        row_range: Range<usize>,
        col_range: Range<usize>,
        sb_size: BlockSize,
    ) -> Result<Self, TileMiSizeStateError> {
        let mi_rows = row_range.end.saturating_sub(row_range.start);
        let mi_cols = col_range.end.saturating_sub(col_range.start);
        let allocation = Self::allocation(mi_rows, mi_cols, sb_size)?;
        let _ = mi_rows
            .checked_mul(mi_cols)
            .ok_or(TileMiSizeStateError::ArithmeticOverflow {
                operation: "mi_rows * mi_cols",
                left: mi_rows,
                right: mi_cols,
            })?;
        Ok(Self {
            origin_row: row_range.start,
            origin_col: col_range.start,
            mi_rows,
            mi_cols,
            mi_sizes: filled_planes(allocation.padded_grid_cells, BLOCK_256X256_INDEX)?,
            mi_size_stride: allocation.padded_cols,
            left_mi_sizes: filled_planes(allocation.padded_rows, CLEAR_PARTITION_CONTEXT)?,
            above_mi_sizes: filled_planes(allocation.padded_cols, CLEAR_PARTITION_CONTEXT)?,
        })
    }

    pub(crate) fn clear_left_context(&mut self) {
        for line in &mut self.left_mi_sizes {
            line.fill(CLEAR_PARTITION_CONTEXT);
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
            self.mi_sizes[LUMA_PLANE].as_slice(),
            self.mi_size_stride,
            [
                self.left_mi_sizes[LUMA_PLANE].as_slice(),
                self.left_mi_sizes[CHROMA_PLANE].as_slice(),
            ],
            [
                self.above_mi_sizes[LUMA_PLANE].as_slice(),
                self.above_mi_sizes[CHROMA_PLANE].as_slice(),
            ],
            self.origin_row,
            self.origin_col,
        )
    }

    fn update_plane_block(
        &mut self,
        plane: usize,
        r: usize,
        c: usize,
        mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        let region = self.validated_region(plane, r, c, mi_size)?;
        let mi_size_index = mi_size.index();
        let above_partition_context = partition_context_above(mi_size_index)?;
        let left_partition_context = partition_context_left(mi_size_index)?;
        let cols = region.col_range();
        for row in region.row_range() {
            let row_start = row * self.mi_size_stride;
            self.mi_sizes[plane][row_start + cols.start..row_start + cols.end].fill(mi_size_index);
            self.left_mi_sizes[plane][row] = left_partition_context;
        }
        self.above_mi_sizes[plane][cols].fill(above_partition_context);
        Ok(())
    }

    fn validated_region(
        &self,
        plane: usize,
        r: usize,
        c: usize,
        mi_size: BlockSize,
    ) -> Result<TileMiSizeRegion, TileMiSizeStateError> {
        let height = mi_size.num_4x4_high()?;
        let width = mi_size.num_4x4_wide()?;
        let absolute_row_end =
            r.checked_add(height)
                .ok_or(TileMiSizeStateError::CoordinateOverflow {
                    coordinate: "row",
                    base: r,
                    offset: height,
                })?;
        let absolute_col_end =
            c.checked_add(width)
                .ok_or(TileMiSizeStateError::CoordinateOverflow {
                    coordinate: "col",
                    base: c,
                    offset: width,
                })?;
        let Some(local_r) = r.checked_sub(self.origin_row) else {
            return Err(TileMiSizeStateError::BlockStartOutOfBounds {
                plane,
                r,
                c,
                mi_rows: self.mi_rows,
                mi_cols: self.mi_cols,
            });
        };
        let Some(local_c) = c.checked_sub(self.origin_col) else {
            return Err(TileMiSizeStateError::BlockStartOutOfBounds {
                plane,
                r,
                c,
                mi_rows: self.mi_rows,
                mi_cols: self.mi_cols,
            });
        };
        if local_r >= self.mi_rows || local_c >= self.mi_cols {
            return Err(TileMiSizeStateError::BlockStartOutOfBounds {
                plane,
                r,
                c,
                mi_rows: self.mi_rows,
                mi_cols: self.mi_cols,
            });
        }
        let plane_grid = &self.mi_sizes[plane];
        let rows = plane_grid.len() / self.mi_size_stride;
        let cols = self.mi_size_stride;
        let row_end = absolute_row_end.saturating_sub(self.origin_row);
        let col_end = absolute_col_end.saturating_sub(self.origin_col);
        if row_end > rows || col_end > cols {
            return Err(TileMiSizeStateError::BlockOutOfBounds {
                plane,
                r,
                c,
                row_end,
                col_end,
                mi_rows: rows,
                mi_cols: cols,
            });
        }
        Ok(TileMiSizeRegion {
            r: local_r,
            c: local_c,
            row_end,
            col_end,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileMiSizeStateAllocation {
    padded_rows: usize,
    padded_cols: usize,
    padded_grid_cells: usize,
    entry_count: usize,
}

impl TileMiSizeStateAllocation {
    #[must_use]
    pub(crate) const fn padded_rows(self) -> usize {
        self.padded_rows
    }

    #[must_use]
    pub(crate) const fn padded_cols(self) -> usize {
        self.padded_cols
    }

    #[must_use]
    pub(crate) const fn padded_grid_cells(self) -> usize {
        self.padded_grid_cells
    }

    #[must_use]
    pub(crate) const fn entry_count(self) -> usize {
        self.entry_count
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TileMiSizeRegion {
    r: usize,
    c: usize,
    row_end: usize,
    col_end: usize,
}

impl TileMiSizeRegion {
    fn row_range(self) -> core::ops::Range<usize> {
        self.r..self.row_end
    }

    fn col_range(self) -> core::ops::Range<usize> {
        self.c..self.col_end
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
    #[error("{table} block size {block_size} is outside 0..{max_exclusive}")]
    PartitionContextBlockSizeOutOfRange {
        table: &'static str,
        block_size: usize,
        max_exclusive: usize,
    },
    #[error("{coordinate} coordinate overflow: {base} + {offset}")]
    CoordinateOverflow {
        coordinate: &'static str,
        base: usize,
        offset: usize,
    },
    #[error(
        "MI-size state plane {plane} block ({r},{c})..({row_end},{col_end}) exceeds {mi_rows}x{mi_cols}"
    )]
    BlockOutOfBounds {
        plane: usize,
        r: usize,
        c: usize,
        row_end: usize,
        col_end: usize,
        mi_rows: usize,
        mi_cols: usize,
    },
    #[error(
        "MI-size state plane {plane} block start ({r},{c}) exceeds visible {mi_rows}x{mi_cols}"
    )]
    BlockStartOutOfBounds {
        plane: usize,
        r: usize,
        c: usize,
        mi_rows: usize,
        mi_cols: usize,
    },
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

fn filled_planes(len: usize, value: usize) -> Result<PlaneBuffers, TileMiSizeStateError> {
    Ok([filled_buffer(len, value)?, filled_buffer(len, value)?])
}

fn filled_buffer(len: usize, value: usize) -> Result<Vec<usize>, TileMiSizeStateError> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(len)?;
    buffer.resize(len, value);
    Ok(buffer)
}

fn partition_context_above(mi_size_index: usize) -> Result<usize, TileMiSizeStateError> {
    partition_context_value(
        "PartitionContextAbove",
        &PARTITION_CONTEXT_ABOVE,
        mi_size_index,
    )
}

fn partition_context_left(mi_size_index: usize) -> Result<usize, TileMiSizeStateError> {
    partition_context_value(
        "PartitionContextLeft",
        &PARTITION_CONTEXT_LEFT,
        mi_size_index,
    )
}

fn partition_context_value(
    table: &'static str,
    values: &'static [usize],
    mi_size_index: usize,
) -> Result<usize, TileMiSizeStateError> {
    values.get(mi_size_index).copied().ok_or(
        TileMiSizeStateError::PartitionContextBlockSizeOutOfRange {
            table,
            block_size: mi_size_index,
            max_exclusive: values.len(),
        },
    )
}

#[cfg(test)]
#[path = "mi_size_state_tests.rs"]
mod tests;
