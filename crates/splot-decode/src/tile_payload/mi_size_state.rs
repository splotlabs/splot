// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.4.1 tile-local MI-size state boundary.
//!
//! Feature tracking: `DECODE-TILE-MI-SIZE-STATE-BOUNDARY`.

use std::collections::TryReserveError;

use super::partition_size::{BlockSize, PartitionSizeError};
use super::partition_traversal::TilePartitionContextState;

// AV2 §9.2 generated block-size table index for the §6.19.2.1 clear-context seed.
const BLOCK_256X256_INDEX: usize = 18;
const PLANE_COUNT: usize = 2;

/// Mutable tile-local MI-size state used by partition contexts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileMiSizeState {
    mi_rows: usize,
    mi_cols: usize,
    mi_sizes: [Vec<Vec<usize>>; PLANE_COUNT],
    left_mi_sizes: [Vec<usize>; PLANE_COUNT],
    above_mi_sizes: [Vec<usize>; PLANE_COUNT],
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
            mi_sizes: [
                filled_grid(allocation.padded_rows, allocation.padded_cols)?,
                filled_grid(allocation.padded_rows, allocation.padded_cols)?,
            ],
            left_mi_sizes: [
                filled_line(allocation.padded_rows)?,
                filled_line(allocation.padded_rows)?,
            ],
            above_mi_sizes: [
                filled_line(allocation.padded_cols)?,
                filled_line(allocation.padded_cols)?,
            ],
        })
    }

    /// Applies AV2 § 5.20.4.1 luma MI-size writes for one block.
    pub(crate) fn update_luma_block(
        &mut self,
        r: usize,
        c: usize,
        mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        self.update_plane_block(0, r, c, mi_size)
    }

    /// Applies AV2 § 5.20.4.1 chroma MI-size writes for caller-supplied chroma facts.
    pub(crate) fn update_chroma_block(
        &mut self,
        chroma_mi_row: usize,
        chroma_mi_col: usize,
        chroma_mi_size: BlockSize,
    ) -> Result<(), TileMiSizeStateError> {
        self.update_plane_block(1, chroma_mi_row, chroma_mi_col, chroma_mi_size)
    }

    /// Builds a short-lived read-only partition-context view over this state.
    pub(crate) fn with_context_state<R>(
        &self,
        f: impl for<'ctx> FnOnce(TilePartitionContextState<'ctx>) -> R,
    ) -> Result<R, TileMiSizeStateError> {
        let mi0_rows = row_slices(&self.mi_sizes[0])?;
        let mi1_rows = row_slices(&self.mi_sizes[1])?;
        Ok(f(TilePartitionContextState::new(
            [&mi0_rows, &mi1_rows],
            [&self.left_mi_sizes[0], &self.left_mi_sizes[1]],
            [&self.above_mi_sizes[0], &self.above_mi_sizes[1]],
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
        for row in region.row_range() {
            for col in region.col_range() {
                self.mi_sizes[plane][row][col] = mi_size_index;
            }
            self.left_mi_sizes[plane][row] = mi_size_index;
        }
        for col in region.col_range() {
            self.above_mi_sizes[plane][col] = mi_size_index;
        }
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
        let rows = self.mi_sizes[plane].len();
        let cols = self
            .mi_sizes
            .get(plane)
            .and_then(|plane_rows| plane_rows.first())
            .map_or(0, Vec::len);
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
    /// Superblock-padded MI row count.
    #[must_use]
    pub(crate) const fn padded_rows(self) -> usize {
        self.padded_rows
    }

    /// Superblock-padded MI column count.
    #[must_use]
    pub(crate) const fn padded_cols(self) -> usize {
        self.padded_cols
    }

    /// MI cells in one padded plane grid.
    #[must_use]
    pub(crate) const fn padded_grid_cells(self) -> usize {
        self.padded_grid_cells
    }

    /// Total `usize` entries allocated across both grids and neighbor lines.
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
    /// State dimensions were empty.
    #[error("MI-size state dimensions must be nonzero, got {mi_rows}x{mi_cols}")]
    EmptyDimensions {
        /// MI rows.
        mi_rows: usize,
        /// MI columns.
        mi_cols: usize,
    },
    /// Allocation arithmetic overflowed.
    #[error("{operation} overflow: left {left}, right {right}")]
    ArithmeticOverflow {
        /// Operation name.
        operation: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
    /// Allocation failed.
    #[error("MI-size state allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    /// A block-size table lookup failed.
    #[error("MI-size state block-size lookup failed: {0}")]
    Size(#[from] PartitionSizeError),
    /// Coordinate addition overflowed.
    #[error("{coordinate} coordinate overflow: {base} + {offset}")]
    CoordinateOverflow {
        /// Coordinate name.
        coordinate: &'static str,
        /// Base coordinate.
        base: usize,
        /// Derived offset.
        offset: usize,
    },
    /// The block footprint exceeded the state dimensions.
    #[error(
        "MI-size state plane {plane} block ({r},{c})..({row_end},{col_end}) exceeds {mi_rows}x{mi_cols}"
    )]
    BlockOutOfBounds {
        /// Plane index, 0 for luma and 1 for chroma.
        plane: usize,
        /// Starting row.
        r: usize,
        /// Starting column.
        c: usize,
        /// Exclusive end row.
        row_end: usize,
        /// Exclusive end column.
        col_end: usize,
        /// State row count.
        mi_rows: usize,
        /// State column count.
        mi_cols: usize,
    },
    /// The block start coordinate was outside visible frame MI dimensions.
    #[error(
        "MI-size state plane {plane} block start ({r},{c}) exceeds visible {mi_rows}x{mi_cols}"
    )]
    BlockStartOutOfBounds {
        /// Plane index, 0 for luma and 1 for chroma.
        plane: usize,
        /// Starting row.
        r: usize,
        /// Starting column.
        c: usize,
        /// Visible frame row count.
        mi_rows: usize,
        /// Visible frame column count.
        mi_cols: usize,
    },
    /// A superblock-padded state dimension overflowed.
    #[error(
        "MI-size state padded {axis} dimension overflow: dimension {dimension}, superblock span {sb_span}"
    )]
    PaddedDimensionOverflow {
        /// Axis name.
        axis: &'static str,
        /// Visible dimension.
        dimension: usize,
        /// Superblock span along this axis.
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

fn filled_grid(rows: usize, cols: usize) -> Result<Vec<Vec<usize>>, TileMiSizeStateError> {
    let mut grid = Vec::new();
    grid.try_reserve_exact(rows)?;
    for _ in 0..rows {
        grid.push(filled_line(cols)?);
    }
    Ok(grid)
}

fn filled_line(len: usize) -> Result<Vec<usize>, TileMiSizeStateError> {
    let mut line = Vec::new();
    line.try_reserve_exact(len)?;
    line.resize(len, BLOCK_256X256_INDEX);
    Ok(line)
}

fn row_slices(grid: &[Vec<usize>]) -> Result<Vec<&[usize]>, TileMiSizeStateError> {
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
                assert_eq!(state.left_mi_size_at(plane, row), BLOCK_256X256);
            }
            for col in 0..3 {
                assert_eq!(state.above_mi_size_at(plane, col), BLOCK_256X256);
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
            assert_eq!(state.left_mi_size_at(0, row), BLOCK_16X8);
        }
        for col in 2..6 {
            assert_eq!(state.above_mi_size_at(0, col), BLOCK_16X8);
        }
        assert_eq!(state.mi_size_at(0, 0, 2), BLOCK_256X256);
        assert_eq!(state.mi_size_at(1, 1, 2), BLOCK_256X256);
        assert_eq!(state.left_mi_size_at(0, 0), BLOCK_256X256);
        assert_eq!(state.above_mi_size_at(0, 1), BLOCK_256X256);
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
            assert_eq!(state.left_mi_size_at(1, row), BLOCK_8X8);
            assert_eq!(state.left_mi_size_at(0, row), BLOCK_256X256);
        }
        for col in 1..3 {
            assert_eq!(state.above_mi_size_at(1, col), BLOCK_8X8);
            assert_eq!(state.above_mi_size_at(0, col), BLOCK_256X256);
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
            assert_eq!(state.left_mi_size_at(0, row), BLOCK_64X64);
        }
        for col in 16..32 {
            assert_eq!(state.above_mi_size_at(0, col), BLOCK_64X64);
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
