// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.4.1 tile-local MI-size state boundary.
//!
//! Feature tracking: `DECODE-TILE-MI-SIZE-STATE-BOUNDARY`.

use std::array;
use std::collections::TryReserveError;

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

type MiSizeRow = Vec<usize>;
type MiSizeGrid = Vec<MiSizeRow>;
type PlaneGrids = [MiSizeGrid; PLANE_COUNT];
type PlaneLines = [MiSizeRow; PLANE_COUNT];

/// Mutable tile-local MI-size state used by partition contexts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileMiSizeState {
    mi_rows: usize,
    mi_cols: usize,
    mi_sizes: PlaneGrids,
    left_mi_sizes: PlaneLines,
    above_mi_sizes: PlaneLines,
}

impl TileMiSizeState {
    /// Computes the padded allocation shape used by this state.
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

    /// Creates a state initialized like AV2 clear-left/above context.
    pub(crate) fn new(
        mi_rows: usize,
        mi_cols: usize,
        sb_size: BlockSize,
    ) -> Result<Self, TileMiSizeStateError> {
        let allocation = Self::allocation(mi_rows, mi_cols, sb_size)?;
        let _ = mi_rows
            .checked_mul(mi_cols)
            .ok_or(TileMiSizeStateError::ArithmeticOverflow {
                operation: "mi_rows * mi_cols",
                left: mi_rows,
                right: mi_cols,
            })?;
        Ok(Self {
            mi_rows,
            mi_cols,
            mi_sizes: filled_grids(allocation.padded_rows, allocation.padded_cols)?,
            left_mi_sizes: filled_lines(allocation.padded_rows)?,
            above_mi_sizes: filled_lines(allocation.padded_cols)?,
        })
    }

    /// Resets the left MI-size partition context for a new superblock row.
    pub(crate) fn clear_left_context(&mut self) {
        for line in &mut self.left_mi_sizes {
            line.fill(CLEAR_PARTITION_CONTEXT);
        }
    }

    /// Applies AV2 § 5.20.4.1 luma MI-size writes for one block.
    pub(crate) fn update_luma_block(
        &mut self,
        r: usize,
        c: usize,
        mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        self.update_plane_block(LUMA_PLANE, r, c, mi_size)
    }

    /// Applies AV2 § 5.20.4.1 chroma MI-size writes for caller-supplied chroma facts.
    pub(crate) fn update_chroma_block(
        &mut self,
        chroma_mi_row: usize,
        chroma_mi_col: usize,
        chroma_mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        self.update_plane_block(CHROMA_PLANE, chroma_mi_row, chroma_mi_col, chroma_mi_size)
    }

    /// Builds a short-lived read-only partition-context view over this state.
    pub(crate) fn with_context_state<R>(
        &self,
        f: impl for<'ctx> FnOnce(TilePartitionContextState<'ctx>) -> R,
    ) -> Result<R, TileMiSizeStateError> {
        let mi_rows = plane_row_slices(&self.mi_sizes)?;
        Ok(f(TilePartitionContextState::new(
            array::from_fn(|plane| mi_rows[plane].as_slice()),
            array::from_fn(|plane| self.left_mi_sizes[plane].as_slice()),
            array::from_fn(|plane| self.above_mi_sizes[plane].as_slice()),
        )))
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
            self.mi_sizes[plane][row][cols.clone()].fill(mi_size_index);
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
        let row_end = r
            .checked_add(height)
            .ok_or(TileMiSizeStateError::CoordinateOverflow {
                coordinate: "row",
                base: r,
                offset: height,
            })?;
        let col_end = c
            .checked_add(width)
            .ok_or(TileMiSizeStateError::CoordinateOverflow {
                coordinate: "col",
                base: c,
                offset: width,
            })?;
        if r >= self.mi_rows || c >= self.mi_cols {
            return Err(TileMiSizeStateError::BlockStartOutOfBounds {
                plane,
                r,
                c,
                mi_rows: self.mi_rows,
                mi_cols: self.mi_cols,
            });
        }
        let plane_grid = &self.mi_sizes[plane];
        let rows = plane_grid.len();
        let cols = plane_grid.first().map_or(0, Vec::len);
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
            r,
            c,
            row_end,
            col_end,
        })
    }

    #[cfg(test)]
    fn mi_size_at(&self, plane: usize, row: usize, col: usize) -> usize {
        self.mi_sizes[plane][row][col]
    }

    #[cfg(test)]
    fn left_mi_size_at(&self, plane: usize, row: usize) -> usize {
        self.left_mi_sizes[plane][row]
    }

    #[cfg(test)]
    fn above_mi_size_at(&self, plane: usize, col: usize) -> usize {
        self.above_mi_sizes[plane][col]
    }
}

/// Padded allocation accounting for [`TileMiSizeState`].
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

/// Error returned by the tile MI-size state boundary.
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

fn filled_grids(rows: usize, cols: usize) -> Result<PlaneGrids, TileMiSizeStateError> {
    Ok([filled_grid(rows, cols)?, filled_grid(rows, cols)?])
}

fn filled_grid(rows: usize, cols: usize) -> Result<MiSizeGrid, TileMiSizeStateError> {
    let mut grid = Vec::new();
    grid.try_reserve_exact(rows)?;
    for _ in 0..rows {
        grid.push(filled_row(cols, BLOCK_256X256_INDEX)?);
    }
    Ok(grid)
}

fn filled_lines(len: usize) -> Result<PlaneLines, TileMiSizeStateError> {
    Ok([filled_line(len)?, filled_line(len)?])
}

fn filled_line(len: usize) -> Result<MiSizeRow, TileMiSizeStateError> {
    filled_row(len, CLEAR_PARTITION_CONTEXT)
}

fn filled_row(len: usize, value: usize) -> Result<MiSizeRow, TileMiSizeStateError> {
    let mut line = Vec::new();
    line.try_reserve_exact(len)?;
    line.resize(len, value);
    Ok(line)
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

fn plane_row_slices(
    grids: &[MiSizeGrid; PLANE_COUNT],
) -> Result<[Vec<&[usize]>; PLANE_COUNT], TileMiSizeStateError> {
    Ok([
        row_slices(&grids[LUMA_PLANE])?,
        row_slices(&grids[CHROMA_PLANE])?,
    ])
}

fn row_slices(grid: &[MiSizeRow]) -> Result<Vec<&[usize]>, TileMiSizeStateError> {
    let mut rows = Vec::new();
    rows.try_reserve_exact(grid.len())?;
    for row in grid {
        rows.push(row.as_slice());
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const BLOCK_4X4: usize = 0;
    const BLOCK_8X8: usize = 3;
    const BLOCK_16X8: usize = 5;
    const BLOCK_64X64: usize = 12;
    const BLOCK_256X256: usize = 18;

    fn block(index: usize) -> BlockSize {
        BlockSize::new(index).unwrap()
    }

    fn new_state(mi_rows: usize, mi_cols: usize) -> TileMiSizeState {
        TileMiSizeState::new(mi_rows, mi_cols, block(BLOCK_64X64)).unwrap()
    }

    #[test]
    fn initializes_luma_and_chroma_with_clear_context_sentinel() {
        let state = new_state(2, 3);

        for plane in 0..2 {
            for row in 0..2 {
                for col in 0..3 {
                    assert_eq!(state.mi_size_at(plane, row, col), BLOCK_256X256);
                }
                assert_eq!(state.left_mi_size_at(plane, row), CLEAR_PARTITION_CONTEXT);
            }
            for col in 0..3 {
                assert_eq!(state.above_mi_size_at(plane, col), CLEAR_PARTITION_CONTEXT);
            }
        }
        assert_eq!(state.mi_sizes[0].len(), 16);
        assert_eq!(state.mi_sizes[0][0].len(), 16);
        assert_eq!(state.left_mi_sizes[0].len(), 16);
        assert_eq!(state.above_mi_sizes[0].len(), 16);
    }

    #[test]
    fn allocation_accounting_includes_superblock_padding_and_neighbor_lines() {
        let allocation = TileMiSizeState::allocation(18, 18, block(BLOCK_64X64)).unwrap();

        assert_eq!(allocation.padded_rows(), 32);
        assert_eq!(allocation.padded_cols(), 32);
        assert_eq!(allocation.padded_grid_cells(), 1024);
        assert_eq!(allocation.entry_count(), 2 * (1024 + 32 + 32));
    }

    #[test]
    fn rejects_empty_dimensions() {
        assert!(matches!(
            TileMiSizeState::new(0, 1, block(BLOCK_64X64)).unwrap_err(),
            TileMiSizeStateError::EmptyDimensions {
                mi_rows: 0,
                mi_cols: 1
            }
        ));
        assert!(matches!(
            TileMiSizeState::new(1, 0, block(BLOCK_64X64)).unwrap_err(),
            TileMiSizeStateError::EmptyDimensions {
                mi_rows: 1,
                mi_cols: 0
            }
        ));
    }

    #[test]
    fn updates_luma_footprint_and_neighbor_lines() {
        let mut state = new_state(6, 6);

        state.update_luma_block(1, 2, block(BLOCK_16X8)).unwrap();

        for row in 1..3 {
            for col in 2..6 {
                assert_eq!(state.mi_size_at(0, row, col), BLOCK_16X8);
            }
            assert_eq!(
                state.left_mi_size_at(0, row),
                partition_context_left(BLOCK_16X8).unwrap()
            );
        }
        for col in 2..6 {
            assert_eq!(
                state.above_mi_size_at(0, col),
                partition_context_above(BLOCK_16X8).unwrap()
            );
        }
        assert_eq!(state.mi_size_at(0, 0, 2), BLOCK_256X256);
        assert_eq!(state.mi_size_at(1, 1, 2), BLOCK_256X256);
        assert_eq!(state.left_mi_size_at(0, 0), CLEAR_PARTITION_CONTEXT);
        assert_eq!(state.above_mi_size_at(0, 1), CLEAR_PARTITION_CONTEXT);
    }

    #[test]
    fn updates_chroma_footprint_without_touching_luma() {
        let mut state = new_state(4, 4);

        state.update_chroma_block(1, 1, block(BLOCK_8X8)).unwrap();

        for row in 1..3 {
            for col in 1..3 {
                assert_eq!(state.mi_size_at(1, row, col), BLOCK_8X8);
                assert_eq!(state.mi_size_at(0, row, col), BLOCK_256X256);
            }
            assert_eq!(
                state.left_mi_size_at(1, row),
                partition_context_left(BLOCK_8X8).unwrap()
            );
            assert_eq!(state.left_mi_size_at(0, row), CLEAR_PARTITION_CONTEXT);
        }
        for col in 1..3 {
            assert_eq!(
                state.above_mi_size_at(1, col),
                partition_context_above(BLOCK_8X8).unwrap()
            );
            assert_eq!(state.above_mi_size_at(0, col), CLEAR_PARTITION_CONTEXT);
        }
    }

    #[test]
    fn accepts_edge_block_footprint_inside_padded_superblock_extent() {
        let mut state = new_state(18, 18);

        state.update_luma_block(16, 16, block(BLOCK_64X64)).unwrap();

        for row in 16..32 {
            for col in 16..32 {
                assert_eq!(state.mi_size_at(0, row, col), BLOCK_64X64);
            }
            assert_eq!(
                state.left_mi_size_at(0, row),
                partition_context_left(BLOCK_64X64).unwrap()
            );
        }
        for col in 16..32 {
            assert_eq!(
                state.above_mi_size_at(0, col),
                partition_context_above(BLOCK_64X64).unwrap()
            );
        }
    }

    #[test]
    fn rejects_start_outside_visible_dimensions_without_mutating_state() {
        let mut state = new_state(2, 2);
        let before = state.clone();

        let err = state.update_luma_block(2, 0, block(BLOCK_4X4)).unwrap_err();

        assert!(matches!(
            err,
            TileMiSizeStateError::BlockStartOutOfBounds {
                plane: 0,
                r: 2,
                c: 0,
                mi_rows: 2,
                mi_cols: 2
            }
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn rejects_footprint_outside_padded_extent_without_mutating_state() {
        let mut state = new_state(16, 16);
        let before = state.clone();

        let err = state
            .update_luma_block(0, 0, block(BLOCK_256X256))
            .unwrap_err();

        assert!(matches!(
            err,
            TileMiSizeStateError::BlockOutOfBounds {
                plane: 0,
                r: 0,
                c: 0,
                row_end: 64,
                col_end: 64,
                mi_rows: 16,
                mi_cols: 16
            }
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn rejects_coordinate_overflow_without_mutating_state() {
        let mut state = new_state(2, 2);
        let before = state.clone();

        let err = state
            .update_chroma_block(usize::MAX, 0, block(BLOCK_4X4))
            .unwrap_err();

        assert!(matches!(
            err,
            TileMiSizeStateError::CoordinateOverflow {
                coordinate: "row",
                base: usize::MAX,
                offset: 1
            }
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn context_state_view_is_available_after_mutation() {
        let mut state = new_state(16, 16);
        state.update_luma_block(0, 0, block(BLOCK_64X64)).unwrap();
        state.update_chroma_block(4, 4, block(BLOCK_8X8)).unwrap();

        let mi0_rows: Vec<&[usize]> = state.mi_sizes[0].iter().map(Vec::as_slice).collect();
        let mi1_rows: Vec<&[usize]> = state.mi_sizes[1].iter().map(Vec::as_slice).collect();
        let expected = TilePartitionContextState::new(
            [&mi0_rows, &mi1_rows],
            [&state.left_mi_sizes[0], &state.left_mi_sizes[1]],
            [&state.above_mi_sizes[0], &state.above_mi_sizes[1]],
        );

        state
            .with_context_state(|context| {
                assert_eq!(context, expected);
            })
            .unwrap();
    }
}
